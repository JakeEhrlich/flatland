//! Fabrication outputs: RS-274X gerbers (X2 attributes), Excellon drills, zip.

pub mod excellon;
pub mod font;
pub mod rs274x;

use crate::error::{Error, Result};
use crate::geom::{self, Ring};
use crate::model::Board;
use crate::schema::{PadType, Side};
use crate::units::Length;
use rs274x::Gerber;
use std::io::Write as _;
use std::path::{Path, PathBuf};

fn side_word(board: &Board, idx: usize) -> &'static str {
    let n = board.layers().len();
    if n == 1 {
        let name = board.layers()[0].to_ascii_lowercase();
        return if name.starts_with('b') { "Bot" } else { "Top" };
    }
    if idx == 0 { "Top" } else if idx == n - 1 { "Bot" } else { "Inr" }
}

fn copper_ext(board: &Board, idx: usize) -> String {
    match side_word(board, idx) {
        "Top" => "gtl".into(),
        "Bot" => "gbl".into(),
        _ => format!("g{}", idx + 1),
    }
}

/// Emit every fabrication file into `dir`; returns the paths written.
pub fn emit(board: &Board, dir: &Path, zip: bool) -> Result<Vec<PathBuf>> {
    let outline = board.outline.as_ref().ok_or_else(|| Error::with_help("cannot emit gerbers without a board outline", "define one with `pcb outline rect W H`"))?;
    let unrouted: usize = board.connectivity()?.iter().map(|(_, i)| i.len().saturating_sub(1)).sum();
    if unrouted > 0 {
        eprintln!("warning: {unrouted} connection(s) are not routed; these gerbers describe an incomplete board (see `pcb status`)");
    }
    std::fs::create_dir_all(dir).map_err(|e| Error::io(format!("could not create `{}`", dir.display()), e))?;
    let name = board.project.name.replace(' ', "_");
    let rules = board.rules();
    let mut files = Vec::new();
    let write = |file: String, text: String| -> Result<PathBuf> {
        let p = dir.join(file);
        std::fs::write(&p, text).map_err(|e| Error::io(format!("could not write `{}`", p.display()), e))?;
        Ok(p)
    };

    // ---- copper layers
    for (idx, layer) in board.layers().iter().enumerate() {
        let mut g = Gerber::new(&format!("Copper,L{},{}", idx + 1, side_word(board, idx)), "Positive");
        // Pours first (their holes are polarity-cleared), then everything else on top.
        // Lowest priority first: a lower-priority pour's fill has the higher-priority
        // pours' boundaries cut out of it as holes, and an LPC clear erases everything
        // drawn before it, so a pour emitted before a lower-priority neighbour would be
        // wiped out by that neighbour's hole (a power island declared before the ground
        // pour vanished from the fab file this way).
        let mut pours: Vec<_> = board.pours()?.iter().filter(|p| &p.pour.layer == layer).collect();
        pours.sort_by_key(|p| p.pour.priority);
        for p in pours {
            g.regions_fractured(&p.copper);
        }
        // Traces as filled regions (flat ends, round joins), the same polygon the
        // DRC and pours see, so a wide link ending in a narrow pad has no cap
        // poking out of it.
        for t in board.project.traces.iter().filter(|t| &t.layer == layer) {
            for r in crate::geom::stroke_flat(&t.points, t.width) {
                if crate::geom::signed_area(&r) > 0.0 {
                    g.region(&r);
                }
            }
        }
        for pad in board.all_pads().filter(|p| p.on_layer(layer) && p.plated) {
            g.region(&pad.copper);
        }
        for t in board.project.texts.iter().filter(|t| &t.layer == layer) {
            for r in board.text_copper(t) {
                if crate::geom::signed_area(&r) > 0.0 {
                    g.region(&r);
                }
            }
        }
        for v in board.project.vias.iter().filter(|v| v.layers.is_empty() || v.layers.iter().any(|l| l == layer)) {
            g.flash_circle(v.at, v.diameter);
        }
        for h in &board.holes {
            if let Some(c) = &h.copper {
                g.region(c);
            }
        }
        files.push(write(format!("{name}-{}.{}", layer.replace('.', "_"), copper_ext(board, idx)), g.finish())?);
    }

    // ---- solder mask & paste (top and bottom)
    for side in [Side::Top, Side::Bottom] {
        let layer = board.stackup().layer_for_side(side).to_string();
        let (sw, ext_mask, ext_paste, ext_silk) = match side {
            Side::Top => ("Top", "gts", "gtp", "gto"),
            Side::Bottom => ("Bot", "gbs", "gbp", "gbo"),
        };
        // Skip the far side of a single-sided board entirely? Keep mask on both sides
        // (through-hole pads exist on both), skip paste/silk where nothing exists.
        let mut mask = Gerber::new(&format!("Soldermask,{sw}"), "Negative");
        let mut paste = Gerber::new(&format!("Paste,{sw}"), "Positive");
        let mut any_paste = false;
        for pad in board.all_pads() {
            let on_side = match pad.pad_type {
                PadType::ThroughHole => true,
                PadType::Smd => pad.on_layer(&layer) && board.instances.iter().any(|i| i.refdes == pad.refdes && i.side() == Some(side)),
            };
            if !on_side {
                continue;
            }
            for r in pad.mask_ring() {
                mask.region(&r);
            }
            if pad.pad_type == PadType::Smd && pad.paste {
                let shrunk = geom::offset(&[pad.copper.clone()], -rules.paste_shrink);
                for r in shrunk {
                    paste.region(&r);
                    any_paste = true;
                }
            }
        }
        if !rules.tent_vias {
            for v in &board.project.vias {
                mask.flash_circle(v.at, v.diameter + rules.mask_expansion * 2);
            }
        }
        for h in &board.holes {
            match &h.copper {
                Some(c) => {
                    for r in geom::offset(&[c.clone()], rules.mask_expansion) {
                        mask.region(&r);
                    }
                }
                None => mask.flash_circle(h.hole.at, h.hole.drill + rules.mask_expansion * 2),
            }
        }
        files.push(write(format!("{name}-{}.{ext_mask}", if side == Side::Top { "F_Mask" } else { "B_Mask" }), mask.finish())?);
        if any_paste {
            files.push(write(format!("{name}-{}.{ext_paste}", if side == Side::Top { "F_Paste" } else { "B_Paste" }), paste.finish())?);
        }

        // ---- silkscreen: footprint art + refdes, clipped away from exposed copper.
        let silk_art = board.silkscreen(side)?;
        if !silk_art.is_empty() {
            let mut silk = Gerber::new(&format!("Legend,{sw}"), "Positive");
            silk.regions_fractured(&silk_art);
            files.push(write(format!("{name}-{}.{ext_silk}", if side == Side::Top { "F_Silkscreen" } else { "B_Silkscreen" }), silk.finish())?);
        }
    }

    // ---- outline
    let mut edge = Gerber::new("Profile,NP", "Positive");
    let mut pts: Ring = outline.clone();
    pts.push(outline[0]);
    edge.polyline(&pts, Length::from_mm(0.1));
    files.push(write(format!("{name}-Edge_Cuts.gko"), edge.finish())?);

    // ---- drills
    let mut pth = excellon::Drill::new(true, board.layers().len());
    let mut npth = excellon::Drill::new(false, board.layers().len());
    for pad in board.all_pads() {
        if let Some(d) = pad.drill {
            let target = if pad.plated { &mut pth } else { &mut npth };
            match pad.drill_length {
                Some(len) if len > d => target.slot(pad.center, d, len, pad.rotation),
                _ => target.hole(pad.center, d),
            }
        }
    }
    for v in &board.project.vias {
        pth.hole(v.at, v.drill);
    }
    for h in &board.holes {
        if h.hole.plated {
            pth.hole(h.hole.at, h.hole.drill);
        } else {
            npth.hole(h.hole.at, h.hole.drill);
        }
    }
    if !pth.is_empty() {
        files.push(write(format!("{name}-PTH.drl"), pth.finish())?);
    }
    if !npth.is_empty() {
        files.push(write(format!("{name}-NPTH.drl"), npth.finish())?);
    }

    // ---- README for the fab
    let mut readme = format!("{}\n\nCopper layers ({}): {}\nBoard thickness: {}\n\nFiles:\n", board.project.name, board.layers().len(), board.layers().join(", "), board.stackup().board_thickness);
    for f in &files {
        readme.push_str(&format!("  {}\n", f.file_name().unwrap().to_string_lossy()));
    }
    files.push(write("README.txt".into(), readme)?);

    if zip {
        let zip_path = dir.join(format!("{name}-gerbers.zip"));
        let file = std::fs::File::create(&zip_path).map_err(|e| Error::io(format!("could not create `{}`", zip_path.display()), e))?;
        let mut z = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for f in &files {
            z.start_file(f.file_name().unwrap().to_string_lossy(), opts).map_err(|e| Error::msg(format!("zip error: {e}")))?;
            let data = std::fs::read(f).map_err(|e| Error::io(format!("could not read `{}`", f.display()), e))?;
            z.write_all(&data).map_err(|e| Error::io("zip write failed", e))?;
        }
        z.finish().map_err(|e| Error::msg(format!("zip error: {e}")))?;
        files.push(zip_path);
    }
    Ok(files)
}
