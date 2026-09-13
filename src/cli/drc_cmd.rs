//! `pcb drc ...`: rule sets, project rules and waivers.

use super::Ctx;
use crate::error::{list_names, suggest, Error, Result};
use crate::schema::*;
use crate::store;
use clap::Subcommand;
use std::path::{Path, PathBuf};

#[derive(Subcommand)]
pub enum DrcCmd {
    /// Reference a rule set file (hash-pinned) from this project.
    Add {
        /// Path to a `pcb-drc/1` JSON file, or a bundled profile name (see `pcb drc profiles`).
        set: String,
    },
    /// Stop using a rule set (by name or path).
    Remove { set: String },
    /// Show active rule sets, project rules and waivers.
    List,
    /// The rule sets bundled with pcb, ready for `pcb drc add`.
    Profiles,
    /// Print a rule (name, severity, selector, check, description).
    Explain { rule: String },
    /// Add a project-local rule.
    #[command(name = "rule")]
    RuleAdd {
        name: String,
        #[arg(long = "for", value_enum)]
        feature: Feature,
        /// The check as JSON, e.g. '{"min_width": 0.3}' or '{"clearance": {"to": "copper", "min": 0.5}}'.
        #[arg(long)]
        check: String,
        /// Filter as JSON, e.g. '{"net": "GND"}'.
        #[arg(long, name = "where")]
        where_: Option<String>,
        #[arg(long, value_enum, default_value = "error")]
        severity: Severity,
        #[arg(long)]
        description: Option<String>,
    },
    /// Remove a project-local rule.
    #[command(name = "unrule")]
    RuleRemove { name: String },
    /// Silence a rule's findings, for the named features (ids, reference designators or nets) or all of them.
    Waive {
        rule: String,
        features: Vec<String>,
        #[arg(long)]
        reason: String,
    },
    /// Drop the waivers of a rule.
    Unwaive { rule: String },
    /// Write a template rule set file to start a new profile from.
    New {
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
    },
}

pub const PROFILES: &[(&str, &str)] = &[
    ("jlcpcb-fr4-2layer", include_str!("../../drc/jlcpcb-fr4-2layer.json")),
    ("jlcpcb-fr4-4layer", include_str!("../../drc/jlcpcb-fr4-4layer.json")),
    ("jlcpcb-aluminium-1layer", include_str!("../../drc/jlcpcb-aluminium-1layer.json")),
];

fn parse_json<T: serde::de::DeserializeOwned>(what: &str, text: &str) -> Result<T> {
    serde_json::from_str(text).map_err(|e| Error::with_help(format!("bad {what} JSON: {e}"), "see `pcb docs drc` for the grammar"))
}

pub fn run_drc(ctx: &Ctx, c: DrcCmd) -> Result<()> {
    match c {
        DrcCmd::Profiles => {
            for (name, text) in PROFILES {
                let set: RuleSet = serde_json::from_str(text).expect("bundled profile parses");
                println!("{name}: {} rule(s) — {}", set.rules.len(), set.description.as_deref().unwrap_or(""));
            }
            println!("\n`pcb drc add <name>` copies a profile into the project directory (drc/<name>.json) and references it.");
            Ok(())
        }
        DrcCmd::Add { set } => {
            let mut loaded = ctx.load()?;
            let project_dir = loaded.path.parent().unwrap_or(Path::new(".")).to_path_buf();
            let path = if let Some((name, text)) = PROFILES.iter().find(|(n, _)| *n == set) {
                let dir = project_dir.join("drc");
                std::fs::create_dir_all(&dir).map_err(|e| Error::msg(format!("cannot create {}: {e}", dir.display())))?;
                let p = dir.join(format!("{name}.json"));
                let current = std::fs::read_to_string(&p).ok();
                if current.as_deref() != Some(*text) {
                    std::fs::write(&p, text).map_err(|e| Error::msg(format!("cannot write {}: {e}", p.display())))?;
                    println!("{} bundled profile at {}", if current.is_some() { "refreshed" } else { "copied" }, p.display());
                }
                p
            } else {
                let p = PathBuf::from(&set);
                if !p.exists() {
                    let names: Vec<&str> = PROFILES.iter().map(|p| p.0).collect();
                    let mut e = Error::msg(format!("no such file or bundled profile: `{set}`"));
                    e = e.help(match suggest(&set, names.iter().copied()) {
                        Some(s) => format!("did you mean `{s}`? bundled profiles are {}", list_names(names.iter().copied())),
                        None => format!("bundled profiles are {}", list_names(names.iter().copied())),
                    });
                    return Err(e);
                }
                p
            };
            let rs: RuleSet = store::read_json(&path, "rule set")?;
            if rs.schema != DRC_SCHEMA {
                return Err(Error::with_help(format!("{} is `{}`, not `{DRC_SCHEMA}`", path.display(), rs.schema), "see `pcb schema drc`"));
            }
            let url = store::relative_url(&path, &loaded.path);
            let blake3 = Some(store::file_blake3(&path)?);
            if let Some(existing) = loaded.project.drc.rulesets.iter_mut().find(|r| r.url == url) {
                if existing.blake3 != blake3 {
                    existing.blake3 = blake3;
                    loaded.save()?;
                    println!("{} re-pinned to the refreshed file", rs.name);
                } else {
                    println!("{} is already active", rs.name);
                }
                return Ok(());
            }
            loaded.project.drc.rulesets.push(FileRef { url: url.clone(), blake3 });
            loaded.save()?;
            println!("added rule set {} ({} rule(s)) from {url}", rs.name, rs.rules.len());
            Ok(())
        }
        DrcCmd::Remove { set } => {
            let mut loaded = ctx.load()?;
            let before = loaded.project.drc.rulesets.len();
            let mut keep = Vec::new();
            for r in loaded.project.drc.rulesets.drain(..) {
                let path = store::local_path(&r.url, &loaded.path, "rule set")?;
                let name = store::read_json::<RuleSet>(&path, "rule set").map(|s| s.name).unwrap_or_default();
                if r.url == set || name == set || path.file_stem().map_or(false, |s| s == set.as_str()) {
                    continue;
                }
                keep.push(r);
            }
            loaded.project.drc.rulesets = keep;
            if loaded.project.drc.rulesets.len() == before {
                return Err(Error::with_help(format!("no rule set `{set}` in this project"), "`pcb drc list` shows the active ones"));
            }
            loaded.save()
        }
        DrcCmd::List => {
            let loaded = ctx.load()?;
            let (sets, own) = crate::drc::active_rules(&loaded.project, &loaded.path)?;
            for (name, set) in &sets {
                println!("{name}: {} rule(s){}", set.rules.len(), if name == "basic" { " (built in)" } else { "" });
                for r in &set.rules {
                    println!("  {:<28} {:<7} {}{}", r.name, r.severity, r.for_, describe_check(&r.check));
                }
            }
            if !own.is_empty() {
                println!("project rules:");
                for r in &own {
                    println!("  {:<28} {:<7} {}{}", r.name, r.severity, r.for_, describe_check(&r.check));
                }
            }
            for w in &loaded.project.drc.waivers {
                println!("waiver: {} {} — {}", w.rule, if w.features.is_empty() { "(all)".to_string() } else { w.features.join(" ") }, w.reason);
            }
            Ok(())
        }
        DrcCmd::Explain { rule } => {
            let loaded = ctx.load()?;
            let (sets, own) = crate::drc::active_rules(&loaded.project, &loaded.path)?;
            let mut found = false;
            for (set, r) in sets.iter().flat_map(|(n, s)| s.rules.iter().map(move |r| (n.clone(), r))).chain(own.iter().map(|r| ("project".to_string(), r))) {
                if r.name == rule {
                    found = true;
                    println!("{} (from {set}), severity {}", r.name, r.severity);
                    if let Some(d) = &r.description {
                        println!("  {d}");
                    }
                    println!("  for: {}{}", r.for_, r.where_.as_ref().map(|w| format!(" where {}", serde_json::to_string(w).unwrap())).unwrap_or_default());
                    println!("  check: {}", serde_json::to_string(&r.check).unwrap());
                }
            }
            if !found {
                let names: Vec<String> = sets.iter().flat_map(|(_, s)| s.rules.iter().map(|r| r.name.clone())).chain(own.iter().map(|r| r.name.clone())).collect();
                let mut e = Error::msg(format!("no rule `{rule}`"));
                if let Some(s) = suggest(&rule, names.iter().map(|s| s.as_str())) {
                    e = e.help(format!("did you mean `{s}`?"));
                } else {
                    e = e.help("`pcb drc list` shows every active rule");
                }
                return Err(e);
            }
            Ok(())
        }
        DrcCmd::RuleAdd { name, feature, check, where_, severity, description } => {
            let mut loaded = ctx.load()?;
            let check: Check = parse_json("check", &check)?;
            let where_: Option<Where> = match where_ {
                Some(w) => Some(parse_json("where", &w)?),
                None => None,
            };
            loaded.project.drc.rules.retain(|r| r.name != name);
            loaded.project.drc.rules.push(Rule { name: name.clone(), description, severity, for_: feature, where_, check });
            loaded.save()?;
            println!("rule {name} added; `pcb check` runs it");
            Ok(())
        }
        DrcCmd::RuleRemove { name } => {
            let mut loaded = ctx.load()?;
            let before = loaded.project.drc.rules.len();
            loaded.project.drc.rules.retain(|r| r.name != name);
            if loaded.project.drc.rules.len() == before {
                return Err(Error::with_help(format!("no project rule `{name}`"), "only rules added with `pcb drc rule` can be removed; rule-set rules are waived instead"));
            }
            loaded.save()
        }
        DrcCmd::Waive { rule, features, reason } => {
            let mut loaded = ctx.load()?;
            let (sets, own) = crate::drc::active_rules(&loaded.project, &loaded.path)?;
            let known: Vec<String> = sets.iter().flat_map(|(_, s)| s.rules.iter().map(|r| r.name.clone())).chain(own.iter().map(|r| r.name.clone())).collect();
            if !known.iter().any(|k| *k == rule) {
                let mut e = Error::msg(format!("no rule `{rule}` to waive"));
                if let Some(s) = suggest(&rule, known.iter().map(|s| s.as_str())) {
                    e = e.help(format!("did you mean `{s}`?"));
                }
                return Err(e);
            }
            loaded.project.drc.waivers.push(Waiver { rule: rule.clone(), features: features.clone(), reason });
            loaded.save()?;
            println!("waived {rule} for {}", if features.is_empty() { "all features".to_string() } else { features.join(" ") });
            Ok(())
        }
        DrcCmd::Unwaive { rule } => {
            let mut loaded = ctx.load()?;
            let before = loaded.project.drc.waivers.len();
            loaded.project.drc.waivers.retain(|w| w.rule != rule);
            if loaded.project.drc.waivers.len() == before {
                return Err(Error::msg(format!("no waivers for `{rule}`")));
            }
            loaded.save()
        }
        DrcCmd::New { path, name } => {
            let name = name.unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "custom".into()));
            let set = RuleSet {
                schema: DRC_SCHEMA.into(),
                name,
                description: Some("Describe the process these limits come from, and when you read them.".into()),
                applies: Some(Applies { layers: vec![2], material: Some("fr4".into()) }),
                rules: vec![
                    Rule { name: "trace-width".into(), description: Some("Minimum trace width for this process.".into()), severity: Severity::Error, for_: Feature::Copper, where_: None, check: Check::MinWidth(Limit::Mm(0.15)) },
                    Rule {
                        name: "copper-spacing".into(),
                        description: None,
                        severity: Severity::Error,
                        for_: Feature::Copper,
                        where_: None,
                        check: Check::Clearance { to: Feature::Copper, relation: Relation::DifferentNet, min: Limit::Mm(0.15) },
                    },
                ],
            };
            store::write_json(&path, &set)?;
            println!("wrote {}; edit it and `pcb drc add {}`", path.display(), path.display());
            Ok(())
        }
    }
}

fn describe_check(c: &Check) -> String {
    match c {
        Check::Clearance { to, relation, min } => format!(": clearance {} to {to} ({})", limit(min), serde_json::to_string(relation).unwrap().trim_matches('"')),
        Check::MinWidth(l) => format!(": min width {}", limit(l)),
        Check::MinGap(l) => format!(": min gap {}", limit(l)),
        Check::Min(m) => format!(": min {}", kv(m)),
        Check::Max(m) => format!(": max {}", kv(m)),
        Check::Inside(f) => format!(": inside {f}"),
        Check::Connected(b) => format!(": connected = {b}"),
        Check::Exists(b) => format!(": exists = {b}"),
        Check::Placed(b) => format!(": placed = {b}"),
        Check::DesignRules(m) => format!(": design rules at least {}", kv(m)),
        Check::EndJunctions(b) => format!(": end-to-end junctions allowed = {b}"),
        Check::DesignatorFormat(b) => format!(": designator format checked = {b}"),
        Check::BomPrefixes(b) => format!(": one designator prefix per BOM row = {b}"),
    }
}

fn limit(l: &Limit) -> String {
    match l {
        Limit::Mm(v) => format!("{v} mm"),
        Limit::Named(n) => n.clone(),
    }
}

fn kv(m: &indexmap::IndexMap<String, f64>) -> String {
    m.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" ")
}
