# pcb-index(1)

## NAME

pcb index, pcb component — create, share and inspect component libraries.

## SYNOPSIS

```
pcb index new DIR [--name NAME]
pcb index add PATH [--pin]
pcb index remove NAME-OR-URL
pcb index list
pcb index register INDEX COMPONENT.json [--name NAME]
pcb index verify
pcb index update [INDEX]

pcb component list [FILTER]
pcb component show NAME
pcb component datasheet NAME
pcb component template PATH [--name NAME]
```

## DESCRIPTION

A **component index** is a directory holding an `index.json` that maps
component names to component files. A **component file** describes pins, an
optional footprint file, a datasheet URL, per-instance parameters and an
optional SPICE model. A **footprint file** describes pads and silkscreen.
Formats: `pcb schema index|component|footprint`.

Every link (project → index, index → component, component → footprint,
component → SPICE library) is a URL plus an optional **blake3** hash of the
target file. URLs are paths relative to the file that contains them (or
`file:`/`~/`/absolute paths; `http(s)://`, `s3://`, `gs://` are recognised
but not fetched yet — the error tells you to mirror the library locally).
Hashes are verified every time a file is loaded; a mismatch is a hard error
that names both hashes and points at `pcb index update`.

A project may use any number of indexes. Component names are looked up
across all of them; when two indexes define the same name the reference must
be qualified as `INDEX:NAME` (the error says so).

## pcb index new DIR [--name NAME]

Creates `DIR/index.json` (empty, named after `DIR` unless `--name`), plus
empty `DIR/components/` and `DIR/footprints/` directories. Does not touch
the project. Follow with `pcb component template`, edit, then
`pcb index register`.

## pcb index add PATH [--pin]

Adds an existing index (file or its directory) to the current project's
`component_indexes`, as a relative URL. All indexes are loaded together to
catch problems immediately.

`--pin`
: Record the index file's blake3 hash in the project. From then on the
  project refuses to load if the index file changes (`pcb index remove` +
  `pcb index add --pin` again to re-pin; `pcb index remove` works even when
  the hash no longer matches). Component and footprint hashes inside the
  index are checked regardless of pinning.

## pcb index remove NAME-OR-URL

Removes the index from the project by its `name` or by the URL stored in
`pcb.json`. Instances that used components from it will fail validation on
the next command until they are removed or the index is re-added.

## pcb index list

For each index: name, URL, component count, `[pinned]`.

## pcb index register INDEX COMPONENT.json [--name NAME]

Adds (or replaces) an entry in an index. `INDEX` is an index file, its
directory, or the name of an index already in the project. The entry's URL
is computed relative to the index file and its blake3 hash recorded; the
component's `description` is copied into the index for `component list`.
`--name` overrides the registered name (default: the component's `name`).
The component file is validated (schema, pin names, spice template
non-empty) but its footprint is not opened here — `pcb index update` fills
the footprint hash.

## pcb index verify

Checks, for every component in every index of the project: the file exists,
its hash matches, the footprint exists and its hash matches. Prints one line
per problem (`basic:led-5mm: hash mismatch (expected …, actual …)`) and
exits 1 if there were any.

## pcb index update [INDEX]

Recomputes hashes after editing files: for each component in the index (or
in all of the project's indexes), the footprint hash inside the component
file is refreshed (rewriting the component file if it changed), then the
component hash in the index. Prints how many hashes changed. Run this
whenever you edit a library file.

## pcb component list [FILTER]

Lists `INDEX:NAME  description` for every component (substring filter on
name or description).

## pcb component show NAME

Prints the component's origin (index, path), description, datasheet URL,
footprint (name, pad count, path — or `none (virtual component)`), pins
(with pad mapping and descriptions), parameters (defaults, `(required)` if
none) and the SPICE template.

## pcb component datasheet NAME

Prints the datasheet's local path or remote URL so you can open it.

## pcb component template PATH [--name NAME]

Writes a starter component file at `PATH` and a footprint file next to it
(`<NAME>-footprint.json`: a two-pad SMD footprint with silkscreen and
courtyard), already linked with a hash. Edit both, then register.

## WRITING COMPONENTS

* `pins[].name` is what users type after the dot (`U1.VCC`); `pad` is the
  footprint pad it lands on (defaults to the pin name). Pin names may not
  contain `.`.
* Parameters without a `default` must be supplied at `pcb add` time.
* SPICE `template` placeholders: `{ref}` (a leading type letter that
  duplicates the refdes's first letter is dropped: `R{ref}` + `R1` → `R1`,
  `R{ref}_esr` + `C1` → `RC1_esr`), `{pin:NAME}`, `{param}`. Multi-line
  templates are fine. Put `.model`/`.subckt` text in `model` (emitted once
  per component type) and library files in `includes`.
* Omit `footprint` for parts that exist only in simulation (sources,
  probes). They cannot be placed and are ignored by routing/gerbers.
* Footprints are drawn from the top, y-up, centred on the origin; pad
  `type` is `smd` or `through_hole`, `shape` is `circle`, `rect`,
  `round_rect` or `oval`; through-hole pads need `drill`.

## EXAMPLES

```
pcb index new ~/libs/mine --name mine
pcb component template ~/libs/mine/components/ntc-10k.json --name ntc-10k
$EDITOR ~/libs/mine/components/ntc-10k.json ~/libs/mine/components/ntc-10k-footprint.json
pcb index register ~/libs/mine ~/libs/mine/components/ntc-10k.json
pcb index add ~/libs/mine --pin
pcb component show mine:ntc-10k
# later, after editing the footprint:
pcb index update mine && pcb index verify
```

## SEE ALSO

pcb(1), pcb-netlist(1), SCHEMA.md.
