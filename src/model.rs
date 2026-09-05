//! The resolved board: project + library turned into concrete geometry.
//! Everything downstream (visualisation, DSN export, gerbers, simulation)
//! works from a [`Board`].

use crate::error::{list_names, suggest, Error, Result};
use crate::geom::{self, Ring, Rings, Transform};
use crate::schema::*;
use crate::store::{LoadedComponent, Library};
use crate::units::{Length, Point};
use indexmap::IndexMap;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Parse `R1.2` into (`R1`, `2`).
pub fn parse_pin_ref(s: &str) -> Result<(&str, &str)> {
    match s.split_once('.') {
        Some((r, p)) if !r.is_empty() && !p.is_empty() => Ok((r, p)),
        _ => Err(Error::with_help(
            format!("`{s}` is not a pin reference"),
            "pins are written `<instance>.<pin>`, e.g. `R1.1` or `U1.VCC`",
        )),
    }
}

#[derive(Clone, Debug)]
pub struct BoardPad {
    pub refdes: String,
    pub pad_name: String,
    /// Pin name(s) landing on this pad (usually one).
    pub pins: Vec<String>,
    pub net: Option<String>,
    pub pad_type: PadType,
    /// Copper layers this pad exists on.
    pub layers: Vec<String>,
    /// Copper outline in board coordinates.
    pub copper: Ring,
    pub center: Point,
    pub rotation: f64,
    pub shape: PadShape,
    pub size: [Length; 2],
    pub corner_radius: Option<Length>,
    pub drill: Option<Length>,
    pub drill_length: Option<Length>,
    pub plated: bool,
    pub paste: bool,
    pub mask_expansion: Length,
    pub thermal_relief: bool,
}

impl BoardPad {
    pub fn on_layer(&self, layer: &str) -> bool {
        self.layers.iter().any(|l| l == layer)
    }
    /// Solder mask opening.
    pub fn mask_ring(&self) -> Rings {
        geom::offset(&[self.copper.clone()], self.mask_expansion)
    }
    /// Four thermal-relief spokes (as rectangles) crossing a gap of `gap`
    /// around this pad, aligned with the pad's rotation.
    pub fn thermal_spokes(&self, gap: Length, width: Length) -> Rings {
        let reach = Length(self.size[0].nm().max(self.size[1].nm()) / 2) + gap + width;
        let t = Transform { mirror_x: false, degrees: self.rotation, translate: self.center };
        vec![
            t.apply_ring(&geom::rect(Point::ORIGIN, reach * 2, width)),
            t.apply_ring(&geom::rect(Point::ORIGIN, width, reach * 2)),
        ]
    }

    /// Drill outline (circle or slot).
    pub fn drill_ring(&self) -> Option<Ring> {
        let d = self.drill?;
        Some(match self.drill_length {
            Some(len) if len > d => {
                let t = Transform { mirror_x: false, degrees: self.rotation, translate: self.center };
                t.apply_ring(&geom::oval(Point::ORIGIN, len, d))
            }
            _ => geom::circle(self.center, d),
        })
    }
}

#[derive(Clone, Debug)]
pub struct PlacedInstance {
    pub refdes: String,
    pub instance: Instance,
    pub component: LoadedComponent,
    pub transform: Option<Transform>,
    pub pads: Vec<BoardPad>,
    /// Silkscreen polylines (board coords, with width) and filled polygons.
    pub silk_lines: Vec<(Vec<Point>, bool, Length)>,
    pub silk_polys: Vec<Ring>,
    pub courtyard: Vec<(Vec<Point>, bool)>,
    /// Refdes label centre on the board, `None` when unplaced or hidden.
    pub label_at: Option<Point>,
}

impl PlacedInstance {
    /// Text height of the refdes label: the placement override or the rule.
    pub fn label_size(&self, rules: &DesignRules) -> Length {
        self.instance.placement.as_ref().and_then(|p| p.label_size).unwrap_or(rules.silk_text_size)
    }
    pub fn side(&self) -> Option<Side> {
        self.instance.placement.as_ref().map(|p| p.side)
    }
    pub fn is_placed(&self) -> bool {
        self.transform.is_some()
    }
    /// Bounding box of pads + courtyard, in board coordinates.
    pub fn bbox(&self) -> Option<(Point, Point)> {
        let mut rings: Vec<Ring> = self.pads.iter().map(|p| p.copper.clone()).collect();
        for (pts, _) in &self.courtyard {
            rings.push(pts.clone());
        }
        for (pts, _, _) in &self.silk_lines {
            rings.push(pts.clone());
        }
        geom::bounds(&rings)
    }
}

#[derive(Clone, Debug)]
pub struct NetInfo {
    pub name: String,
    pub class: Option<String>,
    /// (refdes, pin) pairs.
    pub pins: Vec<(String, String)>,
}

/// A hole that is not part of a footprint.
#[derive(Clone, Debug)]
pub struct BoardHole {
    pub hole: Hole,
    pub ring: Ring,
    /// Copper ring for plated holes.
    pub copper: Option<Ring>,
}

#[derive(Clone, Debug)]
pub struct PourResult {
    pub pour: Pour,
    /// Requested area (before clearance).
    pub outline: Ring,
    /// Filled copper after clearances.
    pub copper: Rings,
}

pub struct Board {
    pub path: PathBuf,
    pub project: Project,
    pub instances: Vec<PlacedInstance>,
    pub nets: Vec<NetInfo>,
    /// pin reference (`R1.1`) -> net name.
    pub pin_nets: HashMap<String, String>,
    pub outline: Option<Ring>,
    pub holes: Vec<BoardHole>,
    pub pours: Vec<PourResult>,
}

impl Board {
    /// Build the board, validating everything referenced by the project.
    pub fn build(project: Project, path: &Path, lib: &Library) -> Result<Board> {
        let rules = project.design_rules.clone();
        let stackup = project.stackup.clone();
        if stackup.copper_layers.is_empty() {
            return Err(Error::with_help(
                "the stackup has no copper layers",
                "set `stackup.copper_layers` in pcb.json, e.g. [\"F.Cu\", \"B.Cu\"] or [\"B.Cu\"] for a single-sided board",
            ));
        }

        // Nets and pin -> net map.
        let mut pin_nets = HashMap::new();
        let mut nets = Vec::new();
        for (name, net) in &project.nets {
            let mut pins = Vec::new();
            for p in &net.pins {
                let (r, pin) = parse_pin_ref(p)?;
                if let Some(other) = pin_nets.insert(p.clone(), name.clone()) {
                    return Err(Error::msg(format!("pin `{p}` is listed in both net `{other}` and net `{name}`")));
                }
                pins.push((r.to_string(), pin.to_string()));
            }
            nets.push(NetInfo { name: name.clone(), class: net.class.clone(), pins });
        }
        for (name, net) in &project.nets {
            if let Some(c) = &net.class {
                if !rules.net_classes.contains_key(c) {
                    return Err(Error::with_help(
                        format!("net `{name}` has class `{c}` but no such class is defined in design_rules.net_classes"),
                        format!("add it with `pcb rules class {c} --width ... --clearance ...`"),
                    ));
                }
            }
        }

        // Instances.
        let mut instances = Vec::new();
        for (refdes, inst) in &project.components {
            let component = lib.component(&inst.component)?;
            for (pname, _) in &inst.parameters {
                if !component.component.parameters.contains_key(pname) {
                    let names: Vec<&str> = component.component.parameters.keys().map(|s| s.as_str()).collect();
                    let mut e = Error::msg(format!(
                        "instance `{refdes}` sets parameter `{pname}` but component `{}` only has {}",
                        component.name,
                        list_names(names.iter().copied())
                    ));
                    if let Some(s) = suggest(pname, names.iter().copied()) {
                        e = e.help(s);
                    }
                    return Err(e);
                }
            }
            let placed = place_instance(refdes, inst, component, &stackup, &rules, &pin_nets)?;
            instances.push(placed);
        }

        // Validate pins referenced by nets exist.
        for net in &nets {
            for (r, pin) in &net.pins {
                let inst = instances.iter().find(|i| &i.refdes == r).ok_or_else(|| {
                    let names: Vec<&str> = instances.iter().map(|i| i.refdes.as_str()).collect();
                    let mut e = Error::msg(format!(
                        "net `{}` references `{r}.{pin}` but there is no instance `{r}`; instances are {}",
                        net.name,
                        list_names(names.iter().copied())
                    ));
                    if let Some(s) = suggest(r, names.iter().copied()) {
                        e = e.help(s);
                    }
                    e
                })?;
                inst.component.pin(pin).map_err(|e| e.help(format!("referenced from net `{}` as `{r}.{pin}`", net.name)))?;
            }
        }

        // Outline.
        let outline = if project.outline.is_empty() {
            None
        } else {
            Some(geom::edges_to_ring(&project.outline, "the board outline")?)
        };

        // Holes.
        let mut holes = Vec::new();
        for h in &project.holes {
            if let Some(n) = &h.net {
                if !project.nets.contains_key(n) {
                    return Err(Error::msg(format!("hole at {} references unknown net `{n}`", h.at)));
                }
                if !h.plated {
                    return Err(Error::msg(format!("hole at {} has a net but is not plated", h.at)));
                }
            }
            let copper = if h.plated {
                let d = h.diameter.unwrap_or(h.drill + rules.hole_annular_ring * 2);
                Some(geom::circle(h.at, d))
            } else {
                None
            };
            holes.push(BoardHole { hole: h.clone(), ring: geom::circle(h.at, h.drill), copper });
        }

        // Layer checks for traces/vias/pours.
        for t in &project.traces {
            check_layer(&stackup, &t.layer, "trace")?;
            if let Some(n) = &t.net {
                if !project.nets.contains_key(n) {
                    return Err(Error::msg(format!("trace on `{}` references unknown net `{n}`", t.layer)));
                }
            }
            if t.points.len() < 2 {
                return Err(Error::msg(format!("a trace on `{}` has fewer than 2 points", t.layer)));
            }
        }
        for v in &project.vias {
            for l in &v.layers {
                check_layer(&stackup, l, "via")?;
            }
            if let Some(n) = &v.net {
                if !project.nets.contains_key(n) {
                    return Err(Error::msg(format!("via at {} references unknown net `{n}`", v.at)));
                }
            }
        }
        for p in &project.pours {
            check_layer(&stackup, &p.layer, &format!("pour `{}`", p.name))?;
            if let Some(n) = &p.net {
                if !project.nets.contains_key(n) {
                    let names: Vec<&str> = project.nets.keys().map(|s| s.as_str()).collect();
                    let mut e = Error::msg(format!("pour `{}` references unknown net `{n}`", p.name));
                    if let Some(s) = suggest(n, names.iter().copied()) {
                        e = e.help(s);
                    }
                    return Err(e);
                }
            }
        }

        let mut board = Board { path: path.to_path_buf(), project, instances, nets, pin_nets, outline, holes, pours: vec![] };
        board.pours = board.compute_pours()?;
        Ok(board)
    }

    pub fn rules(&self) -> &DesignRules {
        &self.project.design_rules
    }
    pub fn stackup(&self) -> &Stackup {
        &self.project.stackup
    }
    pub fn layers(&self) -> &[String] {
        &self.project.stackup.copper_layers
    }

    pub fn instance(&self, refdes: &str) -> Result<&PlacedInstance> {
        self.instances.iter().find(|i| i.refdes == refdes).ok_or_else(|| {
            let names: Vec<&str> = self.instances.iter().map(|i| i.refdes.as_str()).collect();
            let mut e = Error::msg(format!("no instance named `{refdes}`; instances are {}", list_names(names.iter().copied())));
            if let Some(s) = suggest(refdes, names.iter().copied()) {
                e = e.help(s);
            } else {
                e = e.help("add one with `pcb add <refdes> <component>`");
            }
            e
        })
    }

    pub fn net(&self, name: &str) -> Result<&NetInfo> {
        self.nets.iter().find(|n| n.name == name).ok_or_else(|| {
            let names: Vec<&str> = self.nets.iter().map(|n| n.name.as_str()).collect();
            let mut e = Error::msg(format!("no net named `{name}`; nets are {}", list_names(names.iter().copied())));
            if let Some(s) = suggest(name, names.iter().copied()) {
                e = e.help(s);
            }
            e
        })
    }

    pub fn pin_net(&self, refdes: &str, pin: &str) -> Option<&str> {
        self.pin_nets.get(&format!("{refdes}.{pin}")).map(|s| s.as_str())
    }

    pub fn all_pads(&self) -> impl Iterator<Item = &BoardPad> {
        self.instances.iter().flat_map(|i| i.pads.iter())
    }

    /// Every board pad a pin lands on (usually one).
    pub fn pin_pads(&self, refdes: &str, pin: &str) -> Result<Vec<&BoardPad>> {
        let inst = self.instance(refdes)?;
        let p = inst.component.pin(pin)?;
        let names = p.pad_names();
        let pads: Vec<&BoardPad> = inst.pads.iter().filter(|bp| names.contains(&bp.pad_name.as_str())).collect();
        if pads.is_empty() {
            return Err(Error::msg(format!("instance `{refdes}` is not placed, so pin `{pin}` has no board position"))
                .help(format!("place it with `pcb place {refdes} <x>,<y>`")));
        }
        Ok(pads)
    }

    pub fn pad(&self, refdes: &str, pin: &str) -> Result<&BoardPad> {
        let inst = self.instance(refdes)?;
        let p = inst.component.pin(pin)?;
        inst.pads.iter().find(|bp| bp.pad_name == p.pad_name()).ok_or_else(|| {
            Error::msg(format!("instance `{refdes}` is not placed, so pin `{pin}` has no board position"))
                .help(format!("place it with `pcb place {refdes} <x>,<y>`"))
        })
    }

    /// Silkscreen artwork for one side as filled polygons (with holes),
    /// clipped away from exposed copper: footprint graphics and reference
    /// designators minus every solder-mask opening on that side, grown by
    /// half the silk width plus 0.1 mm.
    pub fn silkscreen(&self, side: Side) -> Result<Rings> {
        let rules = self.rules();
        let mut art: Rings = Vec::new();
        for inst in self.instances.iter().filter(|i| i.side() == Some(side)) {
            for (pts, closed, w) in &inst.silk_lines {
                let mut pts = pts.clone();
                if *closed {
                    if let Some(f) = pts.first().copied() {
                        pts.push(f);
                    }
                }
                art.extend(geom::stroke(&pts, *w));
            }
            art.extend(inst.silk_polys.iter().cloned());
            if let Some(at) = inst.label_at {
                for stroke in crate::gerber::font::render(&inst.refdes, at, inst.label_size(rules), side == Side::Bottom) {
                    art.extend(geom::stroke(&stroke, rules.silk_width));
                }
            }
        }
        if art.is_empty() {
            return Ok(vec![]);
        }
        let layer = self.stackup().layer_for_side(side).to_string();
        let mut openings: Rings = Vec::new();
        for pad in self.all_pads() {
            let on_side = match pad.pad_type {
                PadType::ThroughHole => true,
                PadType::Smd => pad.on_layer(&layer) && self.instances.iter().any(|i| i.refdes == pad.refdes && i.side() == Some(side)),
            };
            if on_side {
                openings.extend(pad.mask_ring());
            }
        }
        for v in &self.project.vias {
            openings.push(geom::circle(v.at, v.diameter + rules.mask_expansion * 2));
        }
        for h in &self.holes {
            match &h.copper {
                Some(c) => openings.extend(geom::offset(&[c.clone()], rules.mask_expansion)),
                None => openings.push(geom::circle(h.hole.at, h.hole.drill + rules.mask_expansion * 2)),
            }
        }
        let keep_out = geom::offset(&openings, rules.silk_width / 2 + Length::from_mm(0.1));
        geom::difference(&art, &keep_out)
    }

    /// Board outline shrunk by the edge clearance (copper keep-in).
    pub fn copper_keepin(&self) -> Option<Rings> {
        let o = self.outline.as_ref()?;
        Some(geom::offset(&[o.clone()], -self.rules().edge_clearance))
    }

    /// All copper on `layer` belonging to `net` (None = unassigned), as
    /// (net, rings) pairs; excludes pours.
    pub fn copper_on_layer(&self, layer: &str) -> Vec<(Option<String>, Ring)> {
        let mut out = Vec::new();
        for pad in self.all_pads() {
            if pad.on_layer(layer) {
                out.push((pad.net.clone(), pad.copper.clone()));
            }
        }
        for t in &self.project.traces {
            if t.layer == layer {
                for r in geom::stroke_flat(&t.points, t.width) {
                    out.push((t.net.clone(), r));
                }
            }
        }
        for v in &self.project.vias {
            if v.layers.is_empty() || v.layers.iter().any(|l| l == layer) {
                out.push((v.net.clone(), geom::circle(v.at, v.diameter)));
            }
        }
        for h in &self.holes {
            if let Some(c) = &h.copper {
                out.push((h.hole.net.clone(), c.clone()));
            }
        }
        out
    }

    fn compute_pours(&self) -> Result<Vec<PourResult>> {
        let mut results = Vec::new();
        let rules = self.rules();
        for (i, pour) in self.project.pours.iter().enumerate() {
            let outline = geom::edges_to_ring(&pour.edges, &format!("pour `{}`", pour.name))?;
            let clearance = pour.clearance.unwrap_or(rules.pour_clearance());
            let mut copper: Rings = vec![outline.clone()];
            if let Some(keepin) = self.copper_keepin() {
                copper = geom::intersection(&copper, &keepin)?;
            }
            // Clear other-net copper, unplated holes, drills.
            let mut cut: Rings = Vec::new();
            for (net, ring) in self.copper_on_layer(&pour.layer) {
                if net.is_some() && net == pour.net {
                    continue;
                }
                cut.push(ring);
            }
            for h in &self.holes {
                if h.copper.is_none() || h.hole.net != pour.net {
                    cut.push(h.ring.clone());
                }
            }
            for pad in self.all_pads() {
                if let Some(d) = pad.drill_ring() {
                    if pad.net.is_none() || pad.net != pour.net || !pad.plated {
                        cut.push(d);
                    }
                }
            }
            // Higher-priority pours on the same layer.
            for (j, other) in self.project.pours.iter().enumerate() {
                if j != i && other.layer == pour.layer && other.priority > pour.priority {
                    cut.push(geom::edges_to_ring(&other.edges, &format!("pour `{}`", other.name))?);
                }
            }
            if !cut.is_empty() {
                let cut = geom::offset(&cut, clearance);
                copper = geom::difference(&copper, &cut)?;
            }
            // Slivers narrower than `pour_min_width` (a trace squeezing between
            // two pads leaves 0.1 mm threads of fill on each side) are dropped
            // by a morphological opening: erode, dilate, clip to the original.
            let min_w = rules.pour_min_width();
            copper = geom::open(&copper, min_w)?;
            // Thermal reliefs: same-net pads get a gap ring bridged by spokes.
            // Spokes are clipped to the fill as it stands here, so they never
            // reach into another net's clearance.
            if pour.thermal.unwrap_or(true) && pour.net.is_some() {
                let gap = pour.clearance.unwrap_or(rules.thermal_gap());
                let mut relieved: Rings = Vec::new();
                let mut spokes: Rings = Vec::new();
                for pad in self.all_pads() {
                    if pad.on_layer(&pour.layer) && pad.net == pour.net && pad.thermal_relief {
                        relieved.push(pad.copper.clone());
                        spokes.extend(pad.thermal_spokes(gap, rules.thermal_spoke_width));
                    }
                }
                if !relieved.is_empty() {
                    let spokes = geom::intersection(&spokes, &copper)?;
                    let ring = geom::offset(&relieved, gap);
                    copper = geom::difference(&copper, &ring)?;
                    // The ring cut can leave slivers too (relief ring against a
                    // neighbouring clearance); open again before the spokes go
                    // back in, since spokes are deliberately thin.
                    copper = geom::open(&copper, min_w)?;
                    copper = geom::union(&[copper, spokes].concat())?;
                }
            }
            // Drop islands: filled fragments that touch no copper of the pour's
            // net would be floating copper. Pours without a net keep everything.
            if let Some(net) = &pour.net {
                let anchors: Rings = self
                    .copper_on_layer(&pour.layer)
                    .into_iter()
                    .filter(|(n, _)| n.as_deref() == Some(net.as_str()))
                    .map(|(_, r)| r)
                    .collect();
                let outers: Vec<Ring> = copper.iter().filter(|r| geom::signed_area(r) > 0.0).cloned().collect();
                let holes: Vec<Ring> = copper.iter().filter(|r| geom::signed_area(r) <= 0.0).cloned().collect();
                let mut kept: Rings = Vec::new();
                for o in outers {
                    if anchors.is_empty() || geom::overlaps(&[o.clone()], &anchors) {
                        // Keep the island and the holes that lie inside it.
                        for h in &holes {
                            if h.first().map_or(false, |p| geom::contains(&o, *p)) {
                                kept.push(h.clone());
                            }
                        }
                        kept.push(o);
                    }
                }
                copper = kept;
            }
            results.push(PourResult { pour: pour.clone(), outline, copper });
        }
        Ok(results)
    }

    /// Bounding box of everything on the board (outline, pads, traces).
    pub fn bbox(&self) -> Option<(Point, Point)> {
        let mut rings: Rings = Vec::new();
        if let Some(o) = &self.outline {
            rings.push(o.clone());
        }
        for i in &self.instances {
            if let Some((lo, hi)) = i.bbox() {
                rings.push(vec![lo, hi]);
            }
        }
        for t in &self.project.traces {
            rings.push(t.points.clone());
        }
        for h in &self.holes {
            rings.push(h.ring.clone());
        }
        for p in &self.pours {
            rings.push(p.outline.clone());
        }
        geom::bounds(&rings)
    }

    /// Connectivity per net: groups of pins that are already joined by
    /// copper. Returns, for each net, the list of islands (each a list of
    /// pin refs) — one island means fully routed.
    pub fn connectivity(&self) -> Result<Vec<(String, Vec<Vec<String>>)>> {
        let mut out = Vec::new();
        for net in &self.nets {
            // Collect, per pin, the centres of all its pads (a pin with several
            // solder tabs is connected through the part itself).
            let mut pad_points: Vec<(String, Point, Vec<(Point, Vec<String>)>)> = Vec::new();
            for (r, pin) in &net.pins {
                if let Ok(pads) = self.pin_pads(r, pin) {
                    if let Some(first) = pads.first() {
                        pad_points.push((format!("{r}.{pin}"), first.center, pads.iter().map(|p| (p.center, p.layers.clone())).collect()));
                    }
                }
            }
            if pad_points.len() <= 1 {
                out.push((net.name.clone(), vec![pad_points.into_iter().map(|p| p.0).collect()]));
                continue;
            }
            // Union-find over pads using copper islands per layer, joined by vias.
            let mut parent: Vec<usize> = (0..pad_points.len()).collect();
            fn find(p: &mut Vec<usize>, i: usize) -> usize {
                if p[i] != i {
                    let r = find(p, p[i]);
                    p[i] = r;
                }
                p[i]
            }
            let vias: Vec<&Via> = self.project.vias.iter().filter(|v| v.net.as_deref() == Some(&net.name)).collect();
            // Each via is a virtual node too.
            let n_pads = pad_points.len();
            parent.extend(n_pads..n_pads + vias.len());
            for layer in self.layers() {
                let mut rings: Rings = Vec::new();
                for (_, ring) in self.copper_on_layer(layer).into_iter().filter(|(n, _)| n.as_deref() == Some(&net.name)) {
                    rings.push(ring);
                }
                for p in &self.pours {
                    if p.pour.layer == *layer && p.pour.net.as_deref() == Some(&net.name) {
                        rings.extend(p.copper.iter().cloned());
                    }
                }
                let islands = geom::union(&rings)?;
                // Group only by positive (outer) rings.
                for island in islands.iter().filter(|r| geom::signed_area(r) > 0.0) {
                    let mut first: Option<usize> = None;
                    let mut nodes: Vec<usize> = Vec::new();
                    for (i, (_, _, pads)) in pad_points.iter().enumerate() {
                        if pads.iter().any(|(pt, layers)| layers.iter().any(|l| l == layer) && geom::contains(island, *pt)) {
                            nodes.push(i);
                        }
                    }
                    for (vi, v) in vias.iter().enumerate() {
                        if (v.layers.is_empty() || v.layers.iter().any(|l| l == layer)) && geom::contains(island, v.at) {
                            nodes.push(n_pads + vi);
                        }
                    }
                    for n in nodes {
                        match first {
                            None => first = Some(n),
                            Some(f) => {
                                let (a, b) = (find(&mut parent, f), find(&mut parent, n));
                                parent[a] = b;
                            }
                        }
                    }
                }
            }
            let mut groups: BTreeMap<usize, Vec<String>> = BTreeMap::new();
            for i in 0..n_pads {
                let r = find(&mut parent, i);
                groups.entry(r).or_default().push(pad_points[i].0.clone());
            }
            out.push((net.name.clone(), groups.into_values().collect()));
        }
        Ok(out)
    }

    /// Airwires: for each net, straight lines between islands that still
    /// need connecting (a minimum spanning tree between island centroids).
    pub fn ratsnest(&self) -> Result<Vec<(String, Point, Point)>> {
        let mut wires = Vec::new();
        for (net, islands) in self.connectivity()? {
            if islands.len() <= 1 {
                continue;
            }
            let centroids: Vec<Point> = islands
                .iter()
                .map(|isl| {
                    let pts: Vec<Point> = isl
                        .iter()
                        .filter_map(|p| parse_pin_ref(p).ok().and_then(|(r, pin)| self.pad(r, pin).ok().map(|bp| bp.center)))
                        .collect();
                    pts[0]
                })
                .collect();
            // Prim's MST.
            let n = centroids.len();
            let mut in_tree = vec![false; n];
            in_tree[0] = true;
            for _ in 1..n {
                let mut best: Option<(Length, usize, usize)> = None;
                for a in 0..n {
                    if !in_tree[a] {
                        continue;
                    }
                    for b in 0..n {
                        if in_tree[b] {
                            continue;
                        }
                        let d = centroids[a].distance(centroids[b]);
                        if best.map_or(true, |(bd, _, _)| d < bd) {
                            best = Some((d, a, b));
                        }
                    }
                }
                if let Some((_, a, b)) = best {
                    in_tree[b] = true;
                    wires.push((net.clone(), centroids[a], centroids[b]));
                }
            }
        }
        Ok(wires)
    }
}

fn check_layer(stackup: &Stackup, layer: &str, what: &str) -> Result<()> {
    if !stackup.has_layer(layer) {
        let names: Vec<&str> = stackup.copper_layers.iter().map(|s| s.as_str()).collect();
        let mut e = Error::msg(format!(
            "{what} is on layer `{layer}` but the stackup only has {}",
            list_names(names.iter().copied())
        ));
        if let Some(s) = suggest(layer, names.iter().copied()) {
            e = e.help(s);
        }
        return Err(e);
    }
    Ok(())
}

fn place_instance(
    refdes: &str,
    inst: &Instance,
    component: LoadedComponent,
    stackup: &Stackup,
    rules: &DesignRules,
    pin_nets: &HashMap<String, String>,
) -> Result<PlacedInstance> {
    let transform = inst.placement.as_ref().map(|p| Transform {
        mirror_x: p.side == Side::Bottom,
        degrees: p.rotation,
        translate: p.at,
    });
    let mut pads = Vec::new();
    let mut silk_lines = Vec::new();
    let mut silk_polys = Vec::new();
    let mut courtyard = Vec::new();
    let mut label_at = None;
    if let (Some(t), Some(fp)) = (transform, &component.footprint) {
        let side = inst.placement.as_ref().map(|p| p.side).unwrap_or_default();
        let fp = &fp.footprint;
        // Pad name -> pins.
        let mut pad_pins: IndexMap<&str, Vec<String>> = IndexMap::new();
        for pin in &component.component.pins {
            for pad_name in pin.pad_names() {
                pad_pins.entry(pad_name).or_default().push(pin.name.clone());
            }
        }
        for pad in &fp.pads {
            let pins = pad_pins.get(pad.name.as_str()).cloned().unwrap_or_default();
            let mut net = None;
            for pin in &pins {
                if let Some(n) = pin_nets.get(&format!("{refdes}.{pin}")) {
                    if let Some(prev) = &net {
                        if prev != n {
                            return Err(Error::msg(format!(
                                "pad `{}` of `{refdes}` carries pins on different nets (`{prev}` and `{n}`)",
                                pad.name
                            )));
                        }
                    }
                    net = Some(n.clone());
                }
            }
            let local = match pad.shape {
                PadShape::Circle => geom::circle(Point::ORIGIN, pad.size[0]),
                PadShape::Rect => geom::rect(Point::ORIGIN, pad.size[0], pad.size[1]),
                PadShape::RoundRect => geom::round_rect(
                    Point::ORIGIN,
                    pad.size[0],
                    pad.size[1],
                    pad.corner_radius.unwrap_or(Length(pad.size[0].nm().min(pad.size[1].nm()) / 4)),
                ),
                PadShape::Oval => geom::oval(Point::ORIGIN, pad.size[0], pad.size[1]),
            };
            let pad_t = Transform { mirror_x: false, degrees: pad.rotation, translate: pad.at };
            let local = pad_t.apply_ring(&local);
            let copper = t.apply_ring(&local);
            let layers = match pad.pad_type {
                PadType::Smd => vec![stackup.layer_for_side(side).to_string()],
                PadType::ThroughHole => stackup.copper_layers.clone(),
            };
            pads.push(BoardPad {
                refdes: refdes.to_string(),
                pad_name: pad.name.clone(),
                pins,
                net,
                pad_type: pad.pad_type,
                layers,
                copper,
                center: t.apply(pad.at),
                rotation: t.rotate_angle(pad.rotation),
                shape: pad.shape,
                size: pad.size,
                corner_radius: pad.corner_radius,
                drill: pad.drill,
                drill_length: pad.drill_length,
                plated: pad.is_plated(),
                paste: pad.paste.unwrap_or(pad.pad_type == PadType::Smd),
                mask_expansion: pad.mask_expansion.unwrap_or(rules.mask_expansion),
                thermal_relief: pad.thermal_relief.unwrap_or(true),
            });
        }
        for g in &fp.silkscreen {
            match g {
                Graphic::Polygon { points } => silk_polys.push(t.apply_ring(points)),
                Graphic::Text { .. } => {}
                _ => {
                    let w = g.width().unwrap_or(rules.silk_width);
                    for (pts, closed) in g.polylines() {
                        silk_lines.push((pts.iter().map(|p| t.apply(*p)).collect(), closed, w));
                    }
                }
            }
        }
        for g in &fp.courtyard {
            for (pts, closed) in g.polylines() {
                courtyard.push((pts.iter().map(|p| t.apply(*p)).collect(), closed));
            }
        }
        let p = inst.placement.as_ref().unwrap();
        if !p.label_hidden {
            label_at = Some(t.apply(p.label_at.or(fp.label_at).unwrap_or(Point::ORIGIN)));
        }
    }
    Ok(PlacedInstance {
        refdes: refdes.to_string(),
        instance: inst.clone(),
        component,
        transform,
        pads,
        silk_lines,
        silk_polys,
        courtyard,
        label_at,
    })
}
