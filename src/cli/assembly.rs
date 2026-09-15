//! Assembly outputs: bill of materials and pick-and-place (component
//! placement) lists, in JLCPCB's upload format or a generic CSV.

use super::Ctx;
use crate::error::{Error, Result};
use crate::model::{Board, PlacedInstance};
use crate::schema::Side;
use clap::{Args, ValueEnum};
use indexmap::IndexMap;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// JLCPCB SMT assembly upload format.
    Jlcpcb,
    /// Generic CSV with every field.
    Csv,
}

#[derive(Args)]
pub struct BomArgs {
    #[arg(short, long, value_enum, default_value_t = Format::Jlcpcb)]
    pub format: Format,
    /// Output file (default `build/assembly/<name>-bom.csv`).
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    /// Include parts marked `assembly: false` and parts without an LCSC number.
    #[arg(long)]
    pub all: bool,
}

#[derive(Args)]
pub struct PnpArgs {
    #[arg(short, long, value_enum, default_value_t = Format::Jlcpcb)]
    pub format: Format,
    /// Output file (default `build/assembly/<name>-cpl.csv`).
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    #[arg(long)]
    pub all: bool,
}

/// Assembly facts about one placed instance.
pub struct Part<'a> {
    pub inst: &'a PlacedInstance,
    pub lcsc: Option<String>,
    pub mpn: Option<String>,
    pub manufacturer: Option<String>,
    pub value: String,
    pub footprint: String,
    pub assemble: bool,
    pub rotation_offset: f64,
}

fn meta_str(inst: &PlacedInstance, key: &str) -> Option<String> {
    inst.component.component.metadata.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()).filter(|s| !s.is_empty())
}

pub fn parts(board: &Board) -> Vec<Part<'_>> {
    board
        .instances
        .iter()
        .filter(|i| !i.component.is_virtual())
        .map(|inst| {
            let meta = &inst.component.component.metadata;
            // An instance parameter `lcsc` (generic passives) beats the component's metadata.
            let lcsc = inst.instance.parameters.get("lcsc").cloned().filter(|s| !s.is_empty()).or_else(|| meta_str(inst, "lcsc"));
            let assemble = meta.get("assembly").and_then(|v| v.as_bool()).unwrap_or(true);
            let value = inst
                .instance
                .parameters
                .get("value")
                .cloned()
                .or_else(|| meta_str(inst, "mpn"))
                .unwrap_or_else(|| inst.component.name.clone());
            Part {
                inst,
                lcsc,
                mpn: meta_str(inst, "mpn"),
                manufacturer: meta_str(inst, "manufacturer"),
                value,
                footprint: inst.component.footprint.as_ref().map(|f| f.footprint.name.clone()).unwrap_or_default(),
                assemble,
                rotation_offset: meta.get("rotation_offset").and_then(|v| v.as_f64()).unwrap_or(0.0),
            }
        })
        .collect()
}

fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn write_out(path: &PathBuf, text: &str) -> Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| Error::io(format!("could not create `{}`", p.display()), e))?;
    }
    std::fs::write(path, text).map_err(|e| Error::io(format!("could not write `{}`", path.display()), e))?;
    println!("wrote {}", path.display());
    Ok(())
}

pub fn bom(ctx: &Ctx, a: BomArgs) -> Result<()> {
    let loaded = ctx.load()?;
    let board = ctx.board(&loaded)?;
    let all = parts(&board);
    let mut skipped_dnp = Vec::new();
    let mut missing = Vec::new();
    // Group identical parts: same component, value and LCSC number.
    let mut groups: IndexMap<(String, String, String), Vec<&Part>> = IndexMap::new();
    for p in &all {
        if !p.assemble && !a.all {
            skipped_dnp.push(p.inst.refdes.clone());
            continue;
        }
        if p.lcsc.is_none() {
            missing.push(p.inst.refdes.clone());
            if !a.all && a.format == Format::Jlcpcb {
                continue;
            }
        }
        groups.entry((p.inst.component.name.clone(), p.value.clone(), p.lcsc.clone().unwrap_or_default())).or_default().push(p);
    }
    let mut text = String::new();
    match a.format {
        Format::Jlcpcb => {
            text.push_str("Comment,Designator,Footprint,LCSC Part #\n");
            for ((_, value, lcsc), ps) in &groups {
                let refs: Vec<&str> = ps.iter().map(|p| p.inst.refdes.as_str()).collect();
                text.push_str(&format!("{},{},{},{}\n", csv_field(value), csv_field(&refs.join(",")), csv_field(&ps[0].footprint), csv_field(lcsc)));
            }
        }
        Format::Csv => {
            text.push_str("Designators,Quantity,Component,Value,Footprint,Manufacturer,MPN,LCSC,Assemble,Description\n");
            for ((comp, value, lcsc), ps) in &groups {
                let refs: Vec<&str> = ps.iter().map(|p| p.inst.refdes.as_str()).collect();
                let p0 = ps[0];
                text.push_str(&format!(
                    "{},{},{},{},{},{},{},{},{},{}\n",
                    csv_field(&refs.join(" ")),
                    ps.len(),
                    csv_field(comp),
                    csv_field(value),
                    csv_field(&p0.footprint),
                    csv_field(p0.manufacturer.as_deref().unwrap_or("")),
                    csv_field(p0.mpn.as_deref().unwrap_or("")),
                    csv_field(lcsc),
                    if p0.assemble { "yes" } else { "no" },
                    csv_field(p0.inst.component.component.description.as_deref().unwrap_or(""))
                ));
            }
        }
    }
    // The upload pair stands alone in build/assembly; everything else goes to reference/.
    let name = board.project.name.replace(' ', "_");
    let path = a.output.unwrap_or_else(|| if a.all { loaded.build_dir().join("assembly").join("reference").join(format!("{name}-bom-all.csv")) } else { loaded.build_dir().join("assembly").join(format!("{name}-bom.csv")) });
    write_out(&path, &text)?;
    let n: usize = groups.values().map(|v| v.len()).sum();
    println!("{} part(s) in {} line item(s)", n, groups.len());
    if !skipped_dnp.is_empty() {
        println!("not assembled (assembly: false): {}", skipped_dnp.join(", "));
    }
    if !missing.is_empty() {
        eprintln!(
            "warning: no LCSC part number for {}; {}",
            missing.join(", "),
            if a.all || a.format == Format::Csv { "listed with an empty part number" } else { "left out of the JLCPCB BOM — set `--param lcsc=C…` on the instance or `metadata.lcsc` in the component" }
        );
    }
    Ok(())
}

pub fn pnp(ctx: &Ctx, a: PnpArgs) -> Result<()> {
    let loaded = ctx.load()?;
    let board = ctx.board(&loaded)?;
    let all = parts(&board);
    let mut text = String::new();
    let mut rows = 0;
    let mut unplaced = Vec::new();
    match a.format {
        Format::Jlcpcb => text.push_str("Designator,Mid X,Mid Y,Layer,Rotation\n"),
        Format::Csv => text.push_str("Designator,X,Y,Side,Rotation,Footprint,Value,LCSC\n"),
    }
    // The placement list must name exactly the parts the BOM names: JLCPCB rejects
    // a designator in one file and not the other. Same filter as `bom`.
    let mut left_out: Vec<(String, &str)> = Vec::new();
    let mut hand: String = String::from("# parts not in the BOM or placement list: place these by hand\n# refdes,value,footprint,x,y,side,rotation,why\n");
    for p in &all {
        let why = if !p.assemble { Some("assembly: false") } else if p.lcsc.is_none() && a.format == Format::Jlcpcb { Some("no LCSC number") } else { None };
        if let (Some(why), false) = (why, a.all) {
            left_out.push((p.inst.refdes.clone(), why));
            if let Some(pl) = &p.inst.instance.placement {
                hand.push_str(&format!("{},{},{},{:.4},{:.4},{},{},{why}\n", p.inst.refdes, csv_field(&p.value), csv_field(&p.footprint), pl.at.x.mm(), pl.at.y.mm(), pl.side, fmt_deg(pl.rotation)));
            } else {
                hand.push_str(&format!("{},{},{},,,,,{why} (unplaced)\n", p.inst.refdes, csv_field(&p.value), csv_field(&p.footprint)));
            }
            continue;
        }
        let Some(pl) = &p.inst.instance.placement else {
            unplaced.push(p.inst.refdes.clone());
            continue;
        };
        // Mid X/Y is the centre of the part's copper pads (JLCPCB places at the part
        // centre, and some footprints put their origin at pin 1); rotation is
        // counter-clockwise degrees plus any per-component offset that maps
        // our footprint orientation onto the vendor's reel orientation.
        let rot = (pl.rotation + p.rotation_offset).rem_euclid(360.0);
        let mid = {
            let rings: Vec<crate::geom::Ring> = p.inst.pads.iter().filter(|q| q.plated).map(|q| q.copper.clone()).collect();
            match crate::geom::bounds(&rings) {
                Some((lo, hi)) => crate::units::Point::nm((lo.x.nm() + hi.x.nm()) / 2, (lo.y.nm() + hi.y.nm()) / 2),
                None => pl.at,
            }
        };
        let side = match pl.side {
            Side::Top => "Top",
            Side::Bottom => "Bottom",
        };
        match a.format {
            Format::Jlcpcb => text.push_str(&format!("{},{:.4},{:.4},{},{}\n", p.inst.refdes, mid.x.mm(), mid.y.mm(), side, fmt_deg(rot))),
            Format::Csv => text.push_str(&format!(
                "{},{:.4},{:.4},{},{},{},{},{}\n",
                p.inst.refdes,
                pl.at.x.mm(),
                pl.at.y.mm(),
                side,
                fmt_deg(rot),
                csv_field(&p.footprint),
                csv_field(&p.value),
                p.lcsc.as_deref().unwrap_or("")
            )),
        }
        rows += 1;
    }
    let name = board.project.name.replace(' ', "_");
    let path = a.output.unwrap_or_else(|| if a.all { loaded.build_dir().join("assembly").join("reference").join(format!("{name}-cpl-all.csv")) } else { loaded.build_dir().join("assembly").join(format!("{name}-cpl.csv")) });
    write_out(&path, &text)?;
    println!("{rows} placement(s); origin is the board frame origin (same as the gerbers), y up, rotation counter-clockwise");
    if !left_out.is_empty() {
        let list: Vec<String> = left_out.iter().map(|(r, why)| format!("{r} ({why})")).collect();
        println!("left out, as in the BOM: {}", list.join(", "));
        let side = loaded.build_dir().join("assembly").join("reference").join("hand-placed.txt");
        if let Some(d) = side.parent() {
            std::fs::create_dir_all(d).map_err(|e| Error::io(format!("could not create `{}`", d.display()), e))?;
        }
        std::fs::write(&side, &hand).map_err(|e| Error::io(format!("could not write `{}`", side.display()), e))?;
        println!("their positions are in {}", side.display());
    }
    if a.format == Format::Jlcpcb && !a.all {
        println!("BOM and placement list name the same {rows} part(s)");
    }
    if !unplaced.is_empty() {
        return Err(Error::with_help(
            format!("these parts are not placed: {}", unplaced.join(", ")),
            "place them with `pcb place <refdes> x,y` (the file was written without them)",
        ));
    }
    Ok(())
}

fn fmt_deg(d: f64) -> String {
    if d.fract() == 0.0 { format!("{}", d as i64) } else { format!("{d:.1}") }
}
