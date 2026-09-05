//! Error type. Every user-facing failure carries a message, an optional
//! `help` hint, and (when it comes from a file) a source span so miette can
//! render the offending text.

use miette::{Diagnostic, NamedSource, SourceSpan};
use std::path::Path;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("{message}")]
    #[diagnostic(code(pcb::error))]
    Plain {
        message: String,
        #[help]
        help: Option<String>,
    },

    #[error("{message}")]
    #[diagnostic(code(pcb::file))]
    InFile {
        message: String,
        #[help]
        help: Option<String>,
        #[source_code]
        src: NamedSource<String>,
        #[label("{label}")]
        span: SourceSpan,
        label: String,
    },

    #[error("{context}")]
    #[diagnostic(code(pcb::io))]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
        #[help]
        help: Option<String>,
    },
}

impl Error {
    pub fn msg(message: impl Into<String>) -> Self {
        Error::Plain { message: message.into(), help: None }
    }

    pub fn with_help(message: impl Into<String>, help: impl Into<String>) -> Self {
        Error::Plain { message: message.into(), help: Some(help.into()) }
    }

    pub fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Error::Io { context: context.into(), source, help: None }
    }

    pub fn io_help(context: impl Into<String>, source: std::io::Error, help: impl Into<String>) -> Self {
        Error::Io { context: context.into(), source, help: Some(help.into()) }
    }

    /// An error located in a source file at a byte offset.
    pub fn in_file(
        path: &Path,
        text: &str,
        offset: usize,
        len: usize,
        message: impl Into<String>,
        label: impl Into<String>,
    ) -> Self {
        Error::InFile {
            message: message.into(),
            help: None,
            src: NamedSource::new(path.display().to_string(), text.to_string()),
            span: (offset.min(text.len()), len.min(text.len().saturating_sub(offset))).into(),
            label: label.into(),
        }
    }

    /// An error located in a source file at a (1-based) line and column.
    pub fn at_line_col(
        path: &Path,
        text: &str,
        line: usize,
        col: usize,
        message: impl Into<String>,
        label: impl Into<String>,
    ) -> Self {
        let mut offset = 0;
        for (i, l) in text.split_inclusive('\n').enumerate() {
            if i + 1 == line {
                offset += col.saturating_sub(1).min(l.len());
                break;
            }
            offset += l.len();
        }
        Self::in_file(path, text, offset, 1, message, label)
    }

    pub fn help(mut self, h: impl Into<String>) -> Self {
        match &mut self {
            Error::Plain { help, .. } | Error::InFile { help, .. } | Error::Io { help, .. } => {
                *help = Some(h.into())
            }
        }
        self
    }
}

/// "did you mean" suggestions for a misspelled name.
pub fn suggest<'a>(needle: &str, haystack: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let mut best: Option<(f64, &str)> = None;
    for cand in haystack {
        let score = strsim::jaro_winkler(&needle.to_lowercase(), &cand.to_lowercase());
        if score > 0.8 && best.map_or(true, |(s, _)| score > s) {
            best = Some((score, cand));
        }
    }
    best.map(|(_, c)| format!("did you mean `{c}`?"))
}

/// Format a list of names for an error message, truncated if long.
pub fn list_names<'a>(names: impl IntoIterator<Item = &'a str>) -> String {
    let v: Vec<&str> = names.into_iter().collect();
    if v.is_empty() {
        return "(none)".into();
    }
    let shown: Vec<String> = v.iter().take(20).map(|s| format!("`{s}`")).collect();
    let mut s = shown.join(", ");
    if v.len() > 20 {
        s.push_str(&format!(", … ({} more)", v.len() - 20));
    }
    s
}
