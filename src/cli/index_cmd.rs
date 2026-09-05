//! Component index management and component inspection.

use super::Ctx;
use crate::error::{list_names, Error, Result};
use crate::schema::*;
use crate::store;
use clap::Subcommand;
use std::path::{Path, PathBuf};

#[derive(Subcommand)]
pub enum IndexCmd {
    /// Create a new, empty component index directory (`<dir>/index.json`).
    New {
        dir: PathBuf,
        /// Index name (defaults to the directory name).
        #[arg(long)]
        name: Option<String>,
    },
    /// Add an existing index file to this project.
    Add {
        /// Path to an `index.json` (or its directory).
        path: PathBuf,
        /// Record the index file's blake3 so changes are detected.
        #[arg(long)]
        pin: bool,
    },
    /// Remove an index from this project (by name or URL).
    Remove { name: String },
    /// List the indexes used by this project.
    List,
    /// Register a component file in an index (`pcb index register <index> <component.json>`).
    Register {
        /// Index file or directory (or index name if it belongs to this project).
        index: String,
        /// Component JSON file.
        component: PathBuf,
        /// Name to register it under (default: the component's `name`).
        #[arg(long)]
        name: Option<String>,
    },
    /// Check every hash recorded in the project's indexes.
    Verify,
    /// Recompute the hashes recorded in an index (after editing component files).
    Update {
        /// Index file, directory or name (default: every index in the project).
        index: Option<String>,
    },
}

fn index_path(p: &Path) -> PathBuf {
    if p.is_dir() { p.join("index.json") } else { p.to_path_buf() }
}

/// Resolve an index by project name, path, or directory.
fn find_index(ctx: &Ctx, s: &str) -> Result<PathBuf> {
    let p = Path::new(s);
    if p.exists() {
        return Ok(index_path(p));
    }
    if let Ok(loaded) = ctx.load() {
        for r in &loaded.project.component_indexes {
            let path = store::local_path(&r.url, &loaded.path, "component index")?;
            let ix: ComponentIndex = store::read_json(&path, "component index")?;
            if ix.name == s {
                return Ok(path);
            }
        }
    }
    Err(Error::with_help(
        format!("`{s}` is neither an index file/directory nor the name of an index in this project"),
        "run `pcb index list` to see the project's indexes",
    ))
}

pub fn run_index(ctx: &Ctx, c: IndexCmd) -> Result<()> {
    match c {
        IndexCmd::New { dir, name } => {
            let name = name.unwrap_or_else(|| dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or("components".into()));
            let path = dir.join("index.json");
            if path.exists() {
                return Err(Error::msg(format!("`{}` already exists", path.display())));
            }
            std::fs::create_dir_all(dir.join("components")).map_err(|e| Error::io("could not create index directory", e))?;
            std::fs::create_dir_all(dir.join("footprints")).map_err(|e| Error::io("could not create index directory", e))?;
            store::write_json(&path, &ComponentIndex::new(&name))?;
            println!("created index `{name}` at {}", path.display());
            println!("add component files under {}/components and register them with `pcb index register {} <file>`", dir.display(), dir.display());
            Ok(())
        }
        IndexCmd::Add { path, pin } => {
            let mut loaded = ctx.load()?;
            let ip = index_path(&path);
            let ix: ComponentIndex = store::read_json(&ip, "component index")?;
            if ix.schema != INDEX_SCHEMA {
                return Err(Error::msg(format!("`{}` is not a component index (schema `{}`)", ip.display(), ix.schema)));
            }
            let url = store::relative_url(&ip, &loaded.path);
            if loaded.project.component_indexes.iter().any(|r| r.url == url) {
                return Err(Error::msg(format!("index `{url}` is already in the project")));
            }
            let blake3 = if pin { Some(store::file_blake3(&ip)?) } else { None };
            loaded.project.component_indexes.push(FileRef { url: url.clone(), blake3 });
            // Loading validates all indexes together (duplicate names etc).
            ctx.library(&loaded)?;
            loaded.save()?;
            println!("added index `{}` ({url}) with {} component(s)", ix.name, ix.components.len());
            Ok(())
        }
        IndexCmd::Remove { name } => {
            let mut loaded = ctx.load()?;
            // Read index names leniently (no hash checks) so a broken index can still be removed.
            let mut names = Vec::new();
            for r in &loaded.project.component_indexes {
                let n = store::local_path(&r.url, &loaded.path, "component index")
                    .ok()
                    .and_then(|p| store::read_json::<ComponentIndex>(&p, "component index").ok())
                    .map(|ix| ix.name);
                names.push(n);
            }
            let before = loaded.project.component_indexes.len();
            let keep: Vec<bool> = loaded
                .project
                .component_indexes
                .iter()
                .zip(names.iter())
                .map(|(r, n)| !(r.url == name || n.as_deref() == Some(name.as_str())))
                .collect();
            let mut i = 0;
            loaded.project.component_indexes.retain(|_| {
                i += 1;
                keep[i - 1]
            });
            if loaded.project.component_indexes.len() == before {
                let known: Vec<String> = names.into_iter().flatten().collect();
                return Err(Error::msg(format!("no index named `{name}`; indexes are {}", list_names(known.iter().map(|s| s.as_str())))));
            }
            loaded.save()
        }
        IndexCmd::List => {
            let loaded = ctx.load()?;
            let lib = ctx.library(&loaded)?;
            if lib.indexes.is_empty() {
                println!("no component indexes; add one with `pcb index add <index.json>`");
            }
            for (r, ix) in loaded.project.component_indexes.iter().zip(lib.indexes.iter()) {
                println!(
                    "{}: {} ({} components){}",
                    ix.index.name,
                    r.url,
                    ix.index.components.len(),
                    if r.blake3.is_some() { " [pinned]" } else { "" }
                );
            }
            Ok(())
        }
        IndexCmd::Register { index, component, name } => {
            let ip = find_index(ctx, &index)?;
            let mut ix: ComponentIndex = store::read_json(&ip, "component index")?;
            let comp: Component = store::read_json(&component, "component")?;
            if comp.schema != COMPONENT_SCHEMA {
                return Err(Error::msg(format!("`{}` is not a component file (schema `{}`)", component.display(), comp.schema)));
            }
            store::validate_component(&comp, &component)?;
            let name = name.unwrap_or_else(|| comp.name.clone());
            let url = store::relative_url(&component, &ip);
            let blake3 = store::file_blake3(&component)?;
            ix.components.insert(name.clone(), IndexEntry { file: FileRef { url: url.clone(), blake3: Some(blake3) }, description: comp.description.clone() });
            store::write_json(&ip, &ix)?;
            println!("registered `{name}` -> {url} in index `{}`", ix.name);
            Ok(())
        }
        IndexCmd::Verify => {
            let loaded = ctx.load()?;
            let lib = ctx.library(&loaded)?;
            let mut problems = 0;
            for ix in &lib.indexes {
                for (name, entry) in &ix.index.components {
                    match check_entry(&ix.path, entry) {
                        Ok(msgs) => {
                            for m in msgs {
                                problems += 1;
                                println!("{}:{name}: {m}", ix.index.name);
                            }
                        }
                        Err(e) => {
                            problems += 1;
                            println!("{}:{name}: {e}", ix.index.name);
                        }
                    }
                }
            }
            if problems == 0 {
                println!("all hashes verified");
                Ok(())
            } else {
                Err(Error::with_help(format!("{problems} problem(s) found"), "run `pcb index update` to accept the current files"))
            }
        }
        IndexCmd::Update { index } => {
            let paths: Vec<PathBuf> = match index {
                Some(s) => vec![find_index(ctx, &s)?],
                None => {
                    let loaded = ctx.load()?;
                    loaded.project.component_indexes.iter().map(|r| store::local_path(&r.url, &loaded.path, "component index")).collect::<Result<_>>()?
                }
            };
            for ip in paths {
                let mut ix: ComponentIndex = store::read_json(&ip, "component index")?;
                let mut changed = 0;
                for (name, entry) in ix.components.iter_mut() {
                    let cp = store::local_path(&entry.file.url, &ip, "component file")?;
                    let h = store::file_blake3(&cp).map_err(|e| e.help(format!("component `{name}` in index `{}`", ip.display())))?;
                    if entry.file.blake3.as_deref() != Some(&h) {
                        entry.file.blake3 = Some(h);
                        changed += 1;
                    }
                    // Also refresh the footprint hash inside the component file.
                    let mut comp: Component = store::read_json(&cp, "component")?;
                    if let Some(fr) = &mut comp.footprint {
                        let fp = store::local_path(&fr.url, &cp, "footprint file")?;
                        let fh = store::file_blake3(&fp)?;
                        if fr.blake3.as_deref() != Some(&fh) {
                            fr.blake3 = Some(fh);
                            store::write_json(&cp, &comp)?;
                            entry.file.blake3 = Some(store::file_blake3(&cp)?);
                            changed += 1;
                        }
                    }
                }
                store::write_json(&ip, &ix)?;
                println!("{}: {changed} hash(es) updated", ip.display());
            }
            Ok(())
        }
    }
}

fn check_entry(index_path: &Path, entry: &IndexEntry) -> Result<Vec<String>> {
    let mut msgs = Vec::new();
    let cp = store::local_path(&entry.file.url, index_path, "component file")?;
    if !cp.exists() {
        return Ok(vec![format!("missing file {}", cp.display())]);
    }
    match &entry.file.blake3 {
        None => msgs.push("no hash recorded".into()),
        Some(h) => {
            let actual = store::file_blake3(&cp)?;
            if !actual.eq_ignore_ascii_case(h) {
                msgs.push(format!("hash mismatch (expected {h}, actual {actual})"));
            }
        }
    }
    let comp: Component = store::read_json(&cp, "component")?;
    if let Some(fr) = &comp.footprint {
        let fp = store::local_path(&fr.url, &cp, "footprint file")?;
        if !fp.exists() {
            msgs.push(format!("missing footprint {}", fp.display()));
        } else if let Some(h) = &fr.blake3 {
            let actual = store::file_blake3(&fp)?;
            if !actual.eq_ignore_ascii_case(h) {
                msgs.push(format!("footprint hash mismatch for {}", fp.display()));
            }
        } else {
            msgs.push("footprint has no hash recorded".into());
        }
    }
    Ok(msgs)
}

#[derive(Subcommand)]
pub enum ComponentCmd {
    /// List components from all indexes.
    List {
        /// Filter by substring.
        filter: Option<String>,
    },
    /// Show a component's pins, parameters, footprint and model.
    Show { name: String },
    /// Print the path of a component's datasheet (or open its URL).
    Datasheet { name: String },
    /// Write a template component (+ footprint) file to start from.
    Template {
        /// Output component file path.
        path: PathBuf,
        #[arg(long, default_value = "my-part")]
        name: String,
    },
}

pub fn run_component(ctx: &Ctx, c: ComponentCmd) -> Result<()> {
    match c {
        ComponentCmd::List { filter } => {
            let loaded = ctx.load()?;
            let lib = ctx.library(&loaded)?;
            let mut any = false;
            for ix in &lib.indexes {
                for (name, entry) in &ix.index.components {
                    if filter.as_ref().map_or(true, |f| name.contains(f.as_str()) || entry.description.as_deref().unwrap_or("").contains(f.as_str())) {
                        any = true;
                        println!("{}:{name}  {}", ix.index.name, entry.description.as_deref().unwrap_or(""));
                    }
                }
            }
            if !any {
                println!("no components{}", filter.map(|f| format!(" matching `{f}`")).unwrap_or_default());
            }
            Ok(())
        }
        ComponentCmd::Show { name } => {
            let loaded = ctx.load()?;
            let lib = ctx.library(&loaded)?;
            let c = lib.component(&name)?;
            println!("{}:{}  ({})", c.index_name, c.name, c.path.display());
            if let Some(d) = &c.component.description {
                println!("  {d}");
            }
            if let Some(ds) = &c.component.datasheet {
                println!("  datasheet: {}", ds.url);
            }
            match &c.footprint {
                Some(f) => println!("  footprint: {} ({} pads, {})", f.footprint.name, f.footprint.pads.len(), f.path.display()),
                None => println!("  footprint: none (virtual component)"),
            }
            println!("  pins:");
            for p in &c.component.pins {
                println!("    {}{}{}", p.name, if p.pad_names() != vec![p.name.as_str()] { format!(" -> pad {}", p.pad_names().join("+")) } else { String::new() }, p.description.as_ref().map(|d| format!("  {d}")).unwrap_or_default());
            }
            if !c.component.parameters.is_empty() {
                println!("  parameters:");
                for (k, p) in &c.component.parameters {
                    println!("    {k} = {}{}", p.default.as_deref().unwrap_or("(required)"), p.description.as_ref().map(|d| format!("  {d}")).unwrap_or_default());
                }
            }
            if let Some(s) = &c.component.spice {
                println!("  spice: {}", s.template.replace('\n', "\n         "));
            }
            Ok(())
        }
        ComponentCmd::Datasheet { name } => {
            let loaded = ctx.load()?;
            let lib = ctx.library(&loaded)?;
            let c = lib.component(&name)?;
            match &c.component.datasheet {
                Some(ds) => match store::resolve_url(&ds.url, &c.path) {
                    store::Location::Local(p) => println!("{}", p.display()),
                    store::Location::Remote(u) => println!("{u}"),
                },
                None => return Err(Error::msg(format!("component `{}` has no datasheet", c.name))),
            }
            Ok(())
        }
        ComponentCmd::Template { path, name } => {
            let fp_path = path.with_file_name(format!("{name}-footprint.json"));
            let footprint = Footprint {
                schema: FOOTPRINT_SCHEMA.into(),
                name: format!("{name}-footprint"),
                description: Some("Two-pad SMD footprint template".into()),
                pads: vec![
                    Pad { name: "1".into(), pad_type: PadType::Smd, shape: PadShape::RoundRect, at: crate::units::Point::mm(-0.8, 0.0), size: [crate::units::Length::from_mm(0.9), crate::units::Length::from_mm(1.0)], rotation: 0.0, corner_radius: None, drill: None, drill_length: None, plated: None, paste: None, mask_expansion: None, thermal_relief: None },
                    Pad { name: "2".into(), pad_type: PadType::Smd, shape: PadShape::RoundRect, at: crate::units::Point::mm(0.8, 0.0), size: [crate::units::Length::from_mm(0.9), crate::units::Length::from_mm(1.0)], rotation: 0.0, corner_radius: None, drill: None, drill_length: None, plated: None, paste: None, mask_expansion: None, thermal_relief: None },
                ],
                silkscreen: vec![Graphic::Line { from: crate::units::Point::mm(-0.2, 0.7), to: crate::units::Point::mm(0.2, 0.7), width: None }],
                courtyard: vec![Graphic::Polyline { points: vec![crate::units::Point::mm(-1.5, -0.8), crate::units::Point::mm(1.5, -0.8), crate::units::Point::mm(1.5, 0.8), crate::units::Point::mm(-1.5, 0.8)], width: None }],
                label_at: Some(crate::units::Point::mm(0.0, 1.3)),
            };
            store::write_json(&fp_path, &footprint)?;
            let mut params = indexmap::IndexMap::new();
            params.insert("value".to_string(), Parameter { default: Some("1k".into()), description: Some("resistance".into()) });
            let comp = Component {
                schema: COMPONENT_SCHEMA.into(),
                name: name.clone(),
                description: Some("Template component; edit me".into()),
                datasheet: Some(FileRef::new("https://example.com/datasheet.pdf")),
                footprint: Some(FileRef { url: store::relative_url(&fp_path, &path), blake3: Some(store::file_blake3(&fp_path)?) }),
                pins: vec![Pin { name: "1".into(), pad: None, description: None }, Pin { name: "2".into(), pad: None, description: None }],
                parameters: params,
                spice: Some(SpiceModel { template: "R{ref} {pin:1} {pin:2} {value}".into(), model: None, includes: vec![] }),
                metadata: Default::default(),
            };
            store::write_json(&path, &comp)?;
            println!("wrote {} and {}", path.display(), fp_path.display());
            println!("register it with `pcb index register <index> {}`", path.display());
            Ok(())
        }
    }
}
