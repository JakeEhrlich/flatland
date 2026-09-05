//! A small s-expression reader with byte spans for error reporting.

use crate::error::{Error, Result};
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Atom { text: String, start: usize, end: usize },
    List { items: Vec<Node>, start: usize, end: usize },
}

impl Node {
    pub fn span(&self) -> (usize, usize) {
        match self {
            Node::Atom { start, end, .. } | Node::List { start, end, .. } => (*start, *end),
        }
    }
    pub fn atom(&self) -> Option<&str> {
        match self {
            Node::Atom { text, .. } => Some(text),
            _ => None,
        }
    }
    pub fn items(&self) -> &[Node] {
        match self {
            Node::List { items, .. } => items,
            _ => &[],
        }
    }
    /// Head keyword of a list, e.g. `net` for `(net GND ...)`.
    pub fn head(&self) -> Option<&str> {
        self.items().first().and_then(|n| n.atom())
    }
    pub fn is(&self, head: &str) -> bool {
        self.head().map_or(false, |h| h.eq_ignore_ascii_case(head))
    }
    /// Child lists whose head is `name`.
    pub fn children(&self, name: &str) -> impl Iterator<Item = &Node> {
        let name = name.to_string();
        self.items().iter().filter(move |n| n.is(&name))
    }
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children(name).next()
    }
    /// Atom argument at index `i` (0 = head).
    pub fn arg(&self, i: usize) -> Option<&str> {
        self.items().get(i).and_then(|n| n.atom())
    }
    pub fn args(&self) -> impl Iterator<Item = &str> {
        self.items().iter().skip(1).filter_map(|n| n.atom())
    }
}

/// Parse a whole file into a list of top-level nodes.
pub fn parse(path: &Path, text: &str) -> Result<Vec<Node>> {
    let mut p = Parser { text, bytes: text.as_bytes(), pos: 0, path };
    let mut out = Vec::new();
    loop {
        p.skip_ws();
        if p.pos >= p.bytes.len() {
            break;
        }
        out.push(p.node()?);
    }
    Ok(out)
}

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    pos: usize,
    path: &'a Path,
}

impl<'a> Parser<'a> {
    fn err(&self, at: usize, msg: &str) -> Error {
        Error::in_file(self.path, self.text, at, 1, format!("could not parse s-expression: {msg}"), "here")
    }
    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() {
            let c = self.bytes[self.pos];
            if c.is_ascii_whitespace() {
                self.pos += 1;
            } else if c == b'#' && (self.pos == 0 || self.bytes[self.pos - 1] == b'\n') {
                while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }
    fn node(&mut self) -> Result<Node> {
        self.skip_ws();
        let start = self.pos;
        match self.bytes.get(self.pos) {
            None => Err(self.err(start, "unexpected end of file")),
            Some(b'(') => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws();
                    match self.bytes.get(self.pos) {
                        None => return Err(self.err(start, "unclosed `(`")),
                        Some(b')') => {
                            self.pos += 1;
                            return Ok(Node::List { items, start, end: self.pos });
                        }
                        _ => items.push(self.node()?),
                    }
                }
            }
            Some(b')') => Err(self.err(start, "unexpected `)`")),
            Some(b'"') => {
                self.pos += 1;
                let mut s = String::new();
                loop {
                    match self.bytes.get(self.pos) {
                        None => return Err(self.err(start, "unterminated string")),
                        Some(b'"') => {
                            self.pos += 1;
                            break;
                        }
                        Some(b'\\') if self.pos + 1 < self.bytes.len() => {
                            s.push(self.bytes[self.pos + 1] as char);
                            self.pos += 2;
                        }
                        Some(&c) => {
                            // Collect one UTF-8 char.
                            let ch = self.text[self.pos..].chars().next().unwrap();
                            s.push(ch);
                            self.pos += ch.len_utf8();
                            let _ = c;
                        }
                    }
                }
                Ok(Node::Atom { text: s, start, end: self.pos })
            }
            Some(_) => {
                while self.pos < self.bytes.len() {
                    let c = self.bytes[self.pos];
                    if c.is_ascii_whitespace() || c == b'(' || c == b')' {
                        break;
                    }
                    self.pos += 1;
                }
                Ok(Node::Atom { text: self.text[start..self.pos].to_string(), start, end: self.pos })
            }
        }
    }
}

/// Quote a token for DSN output if needed.
pub fn quote(s: &str) -> String {
    if s.is_empty() || s.chars().any(|c| c.is_whitespace() || c == '(' || c == ')' || c == '"') {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses() {
        let nodes = parse(Path::new("t.dsn"), "(pcb \"a b\" (net GND (pins R1-1 R2-2)))").unwrap();
        assert_eq!(nodes.len(), 1);
        let pcb = &nodes[0];
        assert!(pcb.is("pcb"));
        assert_eq!(pcb.arg(1), Some("a b"));
        let net = pcb.child("net").unwrap();
        assert_eq!(net.arg(1), Some("GND"));
        assert_eq!(net.child("pins").unwrap().args().collect::<Vec<_>>(), vec!["R1-1", "R2-2"]);
        assert!(parse(Path::new("t"), "(a (b)").is_err());
    }
}
