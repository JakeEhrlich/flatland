//! Generate a SPICE netlist from the board for one simulation study.

use crate::error::{list_names, suggest, Error, Result};
use crate::model::Board;
use crate::schema::Simulation;
use crate::store;
use std::collections::BTreeSet;

/// SPICE node name for a net. `GND`/`0`/`ground` map to node 0.
pub fn node_name(net: &str) -> String {
    let l = net.to_ascii_lowercase();
    if l == "gnd" || l == "0" || l == "ground" {
        return "0".into();
    }
    net.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '$' { c } else { '_' }).collect()
}

pub fn generate(board: &Board, sim: &Simulation, name: &str) -> Result<String> {
    let mut out = String::new();
    out.push_str(&format!("* {} — simulation `{name}` ({})\n", board.project.name, sim.analysis.spice_line()));
    out.push_str(&format!(".title {} {name}\n", board.project.name));

    let mut has_ground = board.nets.iter().any(|n| node_name(&n.name) == "0");
    let mut includes: BTreeSet<String> = BTreeSet::new();
    let mut models: Vec<(String, String)> = Vec::new(); // (component name, text)
    let mut elements = Vec::new();
    let mut skipped = Vec::new();
    let mut warnings = Vec::new();

    for inst in &board.instances {
        let comp = &inst.component;
        let Some(spice) = &comp.component.spice else {
            skipped.push(format!("{} ({})", inst.refdes, comp.name));
            continue;
        };
        for inc in &spice.includes {
            let p = store::local_path(&inc.url, &comp.path, "spice include")?;
            if !p.exists() {
                return Err(Error::msg(format!(
                    "component `{}` includes spice library `{}` which does not exist (resolved to `{}`)",
                    comp.name, inc.url, p.display()
                )));
            }
            store::verify_hash(&p, inc.blake3.as_deref(), "spice library")?;
            includes.insert(store::absolute(&p).display().to_string());
        }
        if let Some(m) = &spice.model {
            if !models.iter().any(|(n, _)| n == &comp.name) {
                models.push((comp.name.clone(), m.trim().to_string()));
            }
        }
        // Parameter values: sim override > instance > default.
        let mut params = indexmap::IndexMap::new();
        for (k, p) in &comp.component.parameters {
            let mut v = p.default.clone();
            if let Some(iv) = inst.instance.parameters.get(k) {
                v = Some(iv.clone());
            }
            if let Some(sv) = sim.parameters.get(&format!("{}.{k}", inst.refdes)) {
                v = Some(sv.clone());
            }
            match v {
                Some(v) => {
                    params.insert(k.clone(), v);
                }
                None => {
                    return Err(Error::with_help(
                        format!("instance `{}` has no value for required parameter `{k}`", inst.refdes),
                        format!("set it with `pcb set {} {k}=<value>`", inst.refdes),
                    ))
                }
            }
        }
        let mut text = expand_template(&spice.template, inst, &params, board, &mut warnings, &mut has_ground)?;
        if !text.ends_with('\n') {
            text.push('\n');
        }
        elements.push(text);
    }
    for (k, _) in &sim.parameters {
        let (r, p) = k.split_once('.').ok_or_else(|| Error::with_help(format!("simulation parameter `{k}` is not `<instance>.<param>`"), "e.g. `R1.value=1k`"))?;
        let inst = board.instance(r).map_err(|e| e.help(format!("referenced by simulation `{name}` parameter `{k}`")))?;
        if !inst.component.component.parameters.contains_key(p) {
            let names: Vec<&str> = inst.component.component.parameters.keys().map(|s| s.as_str()).collect();
            let mut e = Error::msg(format!("simulation `{name}` overrides `{k}` but `{}` has no parameter `{p}` (it has {})", inst.component.name, list_names(names.iter().copied())));
            if let Some(s) = suggest(p, names.iter().copied()) {
                e = e.help(s);
            }
            return Err(e);
        }
    }
    if elements.is_empty() {
        return Err(Error::with_help(
            "nothing to simulate: no instance has a spice model",
            "give components a `spice` section (see `pcb schema component`) and add sources such as `vsource`",
        ));
    }
    if !has_ground {
        return Err(Error::with_help(
            "the circuit has no ground: no net is named GND (or 0)",
            "rename the ground net with `pcb net rename <net> GND`, or connect a pin with `pcb connect <pin> --net GND`",
        ));
    }

    if let Some(t) = sim.temperature {
        out.push_str(&format!(".temp {t}\n"));
    }
    let mut options: Vec<String> = Vec::new();
    if let Some(t) = sim.nominal_temperature {
        options.push(format!("tnom={t}"));
    }
    for (k, v) in &sim.options {
        options.push(if v.is_empty() { k.clone() } else { format!("{k}={v}") });
    }
    if !options.is_empty() {
        out.push_str(&format!(".options {}\n", options.join(" ")));
    }
    for inc in &includes {
        out.push_str(&format!(".include \"{inc}\"\n"));
    }
    for (n, m) in &models {
        out.push_str(&format!("* model for {n}\n{m}\n"));
    }
    out.push_str("* elements\n");
    for e in &elements {
        out.push_str(e);
    }
    if !sim.extra.is_empty() {
        out.push_str("* extra\n");
        for l in &sim.extra {
            out.push_str(l);
            out.push('\n');
        }
    }
    out.push_str(&sim.analysis.spice_line());
    out.push('\n');
    out.push_str(".save all\n");
    for p in &sim.probes {
        if p.starts_with('@') {
            out.push_str(&format!(".save {p}\n"));
        }
    }
    out.push_str(".end\n");
    if !skipped.is_empty() {
        out.insert_str(0, &format!("* not simulated (no spice model): {}\n", skipped.join(", ")));
    }
    for w in warnings {
        out.insert_str(0, &format!("* warning: {w}\n"));
    }
    Ok(out)
}

fn expand_template(
    template: &str,
    inst: &crate::model::PlacedInstance,
    params: &indexmap::IndexMap<String, String>,
    board: &Board,
    warnings: &mut Vec<String>,
    has_ground: &mut bool,
) -> Result<String> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('}').ok_or_else(|| {
            Error::msg(format!("spice template of `{}` has an unclosed `{{`: {template}", inst.component.name))
        })?;
        let key = &after[..end];
        rest = &after[end + 1..];
        if key == "ref" {
            // `R{ref}` with refdes `R1` yields `R1`, not `RR1`: when the
            // template's type letter starts a token and matches the refdes's
            // first letter, drop the duplicate so probes like `@d1[id]` work.
            let first = inst.refdes.chars().next().unwrap_or(' ');
            let mut chars = out.chars().rev();
            let last = chars.next();
            let before = chars.next();
            if let Some(l) = last {
                if l.eq_ignore_ascii_case(&first) && l.is_ascii_alphabetic() && before.map_or(true, |b| b.is_whitespace()) {
                    out.pop();
                }
            }
            out.push_str(&inst.refdes);
        } else if let Some(pin) = key.strip_prefix("pin:") {
            inst.component.pin(pin).map_err(|e| e.help(format!("referenced as `{{pin:{pin}}}` in the spice template of `{}`", inst.component.name)))?;
            match board.pin_net(&inst.refdes, pin) {
                Some(net) => {
                    let n = node_name(net);
                    if n == "0" {
                        *has_ground = true;
                    }
                    out.push_str(&n);
                }
                None => {
                    warnings.push(format!("{}.{pin} is unconnected; it floats as node NC_{}_{pin}", inst.refdes, inst.refdes));
                    out.push_str(&format!("NC_{}_{}", inst.refdes, node_name(pin)));
                }
            }
        } else if let Some(v) = params.get(key) {
            out.push_str(v);
        } else {
            let mut names: Vec<String> = params.keys().cloned().collect();
            names.push("ref".into());
            names.extend(inst.component.component.pins.iter().map(|p| format!("pin:{}", p.name)));
            let mut e = Error::msg(format!(
                "spice template of `{}` uses `{{{key}}}` but the available placeholders are {}",
                inst.component.name,
                list_names(names.iter().map(|s| s.as_str()))
            ));
            if let Some(s) = suggest(key, names.iter().map(|s| s.as_str())) {
                e = e.help(s);
            }
            return Err(e);
        }
    }
    out.push_str(rest);
    Ok(out)
}
