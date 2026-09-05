//! Import a Specctra session (.ses) produced by freerouting: routed wires
//! and vias per net.

use super::sexpr::{parse, Node};
use crate::error::{Error, Result};
use crate::schema::{Trace, Via};
use crate::units::{Length, Point};
use std::path::Path;

pub struct Session {
    pub traces: Vec<Trace>,
    pub vias: Vec<Via>,
}

struct Scale {
    /// Multiply a file coordinate by this to get nanometres.
    nm_per_unit: f64,
}

impl Scale {
    fn from(unit: &str, resolution: f64) -> Result<Scale> {
        let nm_per_base = match unit.to_ascii_lowercase().as_str() {
            "um" => 1_000.0,
            "mm" => 1_000_000.0,
            "cm" => 10_000_000.0,
            "mil" => 25_400.0,
            "inch" | "in" => 25_400_000.0,
            other => return Err(Error::msg(format!("session file uses unknown unit `{other}`"))),
        };
        Ok(Scale { nm_per_unit: nm_per_base / resolution })
    }
    fn len(&self, v: f64) -> Length {
        Length((v * self.nm_per_unit).round() as i64)
    }
}

fn num(n: &Node, path: &Path, text: &str, what: &str) -> Result<f64> {
    let s = n.atom().ok_or_else(|| Error::in_file(path, text, n.span().0, 1, format!("expected a number for {what}"), "here"))?;
    s.parse::<f64>().map_err(|_| Error::in_file(path, text, n.span().0, s.len(), format!("`{s}` is not a number ({what})"), "here"))
}

pub fn parse_session(path: &Path, text: &str, layers: &[String]) -> Result<Session> {
    let nodes = parse(path, text)?;
    let session = nodes
        .iter()
        .find(|n| n.is("session"))
        .ok_or_else(|| Error::msg(format!("`{}` has no (session ...) block; is it a Specctra .ses file?", path.display())))?;
    let routes = session
        .child("routes")
        .ok_or_else(|| Error::with_help(format!("`{}` has no (routes ...) block", path.display()), "freerouting writes routes only when it completes; check its log"))?;
    let mut unit = "um".to_string();
    let mut resolution = 1.0;
    if let Some(r) = routes.child("resolution").or_else(|| session.child("resolution")) {
        unit = r.arg(1).unwrap_or("um").to_string();
        resolution = r.items().get(2).map(|n| num(n, path, text, "resolution")).transpose()?.unwrap_or(1.0);
    }
    let scale = Scale::from(&unit, resolution)?;

    // Via padstacks: name -> (diameter, drill, layers)
    let mut via_defs: Vec<(String, Length, Length, Vec<String>)> = Vec::new();
    if let Some(lib) = routes.child("library_out") {
        for ps in lib.children("padstack") {
            let name = ps.arg(1).unwrap_or("").to_string();
            let mut dia = Length::ZERO;
            let mut lays = Vec::new();
            for shape in ps.children("shape") {
                if let Some(inner) = shape.items().get(1) {
                    if inner.is("circle") {
                        if let Some(l) = inner.arg(1) {
                            lays.push(l.to_string());
                        }
                        if let Some(d) = inner.items().get(2) {
                            dia = scale.len(num(d, path, text, "via diameter")?);
                        }
                    } else if let Some(l) = inner.arg(1) {
                        lays.push(l.to_string());
                    }
                }
            }
            let (d2, drill) = super::emit::parse_via_name(&name).unwrap_or((dia, Length(dia.nm() / 2)));
            if dia.is_zero() {
                dia = d2;
            }
            via_defs.push((name, dia, drill, lays));
        }
    }

    let mut traces = Vec::new();
    let mut vias = Vec::new();
    let net_out = routes.child("network_out").ok_or_else(|| Error::msg("session has no (network_out ...) block"))?;
    for net in net_out.children("net") {
        let net_name = net.arg(1).unwrap_or("").to_string();
        for wire in net.children("wire") {
            let Some(p) = wire.child("path") else { continue };
            let layer = p.arg(1).unwrap_or("").to_string();
            if !layers.iter().any(|l| l == &layer) {
                return Err(Error::in_file(path, text, p.span().0, 1, format!("wire on unknown layer `{layer}` (board layers: {})", layers.join(", ")), "here"));
            }
            let width = scale.len(num(&p.items()[2], path, text, "wire width")?);
            let nums: Vec<f64> = p.items()[3..].iter().filter(|n| n.atom().is_some()).map(|n| num(n, path, text, "coordinate")).collect::<Result<_>>()?;
            let mut pts = Vec::new();
            for c in nums.chunks(2) {
                if c.len() == 2 {
                    pts.push(Point::new(scale.len(c[0]), scale.len(c[1])));
                }
            }
            pts.dedup();
            if pts.len() >= 2 {
                traces.push(Trace { layer, net: Some(net_name.clone()), width, points: pts, routed: true });
            }
        }
        for via in net.children("via") {
            let name = via.arg(1).unwrap_or("").to_string();
            let x = num(&via.items()[2], path, text, "via x")?;
            let y = num(&via.items()[3], path, text, "via y")?;
            let def = via_defs.iter().find(|d| d.0 == name);
            let (dia, drill, lays) = match def {
                Some((_, d, h, l)) => (*d, *h, l.clone()),
                None => match super::emit::parse_via_name(&name) {
                    Some((d, h)) => (d, h, vec![]),
                    None => return Err(Error::in_file(path, text, via.span().0, 1, format!("via uses unknown padstack `{name}`"), "here")),
                },
            };
            let all = lays.len() >= layers.len() || lays.is_empty();
            vias.push(Via { at: Point::new(scale.len(x), scale.len(y)), net: Some(net_name.clone()), drill, diameter: dia, layers: if all { vec![] } else { lays }, routed: true });
        }
    }
    Ok(Session { traces, vias })
}
