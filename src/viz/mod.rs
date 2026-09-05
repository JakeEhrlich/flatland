//! Visualisation: SVG generation and PNG rasterisation.

pub mod netlist;
pub mod pcb;
pub mod svg;

use crate::error::{Error, Result};
use std::path::Path;

/// Rasterise an SVG string to a PNG of the given pixel width.
pub fn svg_to_png(svg: &str, width: u32) -> Result<Vec<u8>> {
    let mut opt = usvg::Options::default();
    opt.fontdb_mut().load_system_fonts();
    opt.font_family = "Helvetica".into();
    let tree = usvg::Tree::from_str(svg, &opt).map_err(|e| Error::msg(format!("internal error: generated SVG is invalid: {e}")))?;
    let size = tree.size();
    let scale = width as f32 / size.width();
    let height = (size.height() * scale).ceil().max(1.0) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width.max(1), height)
        .ok_or_else(|| Error::msg(format!("cannot allocate a {width}x{height} image")))?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.encode_png().map_err(|e| Error::msg(format!("PNG encoding failed: {e}")))
}

/// Write `<path>.png` and the SVG next to it (same stem, `.svg`).
pub fn write_svg_png(svg: &str, png_path: &Path, width: u32) -> Result<()> {
    if let Some(parent) = png_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(format!("could not create `{}`", parent.display()), e))?;
    }
    let svg_path = png_path.with_extension("svg");
    std::fs::write(&svg_path, svg).map_err(|e| Error::io(format!("could not write `{}`", svg_path.display()), e))?;
    let png = svg_to_png(svg, width)?;
    std::fs::write(png_path, png).map_err(|e| Error::io(format!("could not write `{}`", png_path.display()), e))?;
    println!("wrote {} and {}", png_path.display(), svg_path.display());
    Ok(())
}
