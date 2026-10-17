# The Spry Bible

The product bible says what Spry is, who it's for, and how we'll know each
feature works. Engineering rules live in [`../CLAUDE.md`](../CLAUDE.md).
Every feature below has Given/When/Then acceptance scenarios, and each
scenario has a matching test in `tests/features/`. A feature counts as done
only when all of its scenarios pass.

## The brief, word for word

> using rust or go, build a cross platform sprite and animation and pixel art
> editor that supports mspaint style dithers, llm mcp, llm static diffusion
> with micro pixel art models, clones, aseprite features, an intuivie modern
> apple glass like ui, previews, exports, automatic sprite sheeting,
> versioning, notes, and any other useful and innovative features.

> use pivotal thoughtbot style opoinionated consultant, high test coverage,
> bdd e2e style, no placeholder code, only working code, literate and
> expressive code, write it in rules and bibles along with my roiginal prompt.

## Vision

Spry is a pixel art studio that's quick to pick up and deep enough for a
working game artist. The pixels belong to you. AI is a collaborator you can
hand the brush to: an LLM drives the editor through MCP, and diffusion models
sketch ideas that Spry snaps to your palette and grid. Nothing is ever lost:
every save is a version you can browse, compare and restore.

**For:** indie game artists, hobbyist pixel artists, and developers who want
sprites without leaving their toolchain.

**Principles**
1. *Pixels are sacred.* Every operation, AI ones included, ends on the
   document's grid and palette. No blurry anti-aliased output.
2. *The canvas is the hero.* The interface floats over it as frosted-glass
   panels that get out of the way.
3. *Humans and LLMs use the same verbs.* Anything the toolbar can do, an MCP
   tool can do, through the same command.
4. *Undo everything.* Every change is a command, and every command can be
   undone.

## Decisions (and why)

| Decision | Choice | Why |
|---|---|---|
| Language | Rust | One native binary on Linux/macOS/Windows, fast pixel loops, strong types for a document model. |
| UI | egui + eframe (glow) | Immediate mode suits a canvas tool. Runs on every desktop OS, and we can restyle it into glass. |
| File format | `.spry` = gzip'd JSON, cels as base64 PNG | Easy to inspect, diff and extend. Version history lives in the file. |
| Sheet metadata | Aseprite JSON ("hash" layout) | Engines (Godot, Unity, Phaser, Love2D) already read it. |
| MCP transport | stdio (`spry mcp`) and local HTTP from the running app | stdio for headless agents (Claude Code, Claude Desktop); HTTP so an agent can paint into the window you have open. |
| Diffusion | Automatic1111/Forge and ComfyUI HTTP APIs | People already run these locally with pixel-art LoRAs and small checkpoints. |
| Chat LLM | Claude (Messages API, raw HTTP) and Ollama | A strong hosted model and a private local one. |

## Glossary

- **Document**: one sprite. Canvas size, palette, layers, frames, tags,
  notes and history.
- **Layer**: a named, ordered stack entry with visibility, opacity and a lock.
- **Frame**: a moment in the animation, with a duration in milliseconds.
- **Cel**: the image of one layer at one frame. It can be empty.
- **Clone (linked cel)**: a cel that shares its pixels with other cels.
  Painting one paints all of them.
- **Tag**: a named frame range with a loop direction (forward, reverse,
  ping-pong), e.g. "walk" = frames 1–4.
- **Palette**: an ordered list of colours. **Ramp**: a run of palette colours
  from dark to light, which shading moves along.
- **Dither pattern**: an 8×8 on/off mask that picks primary or secondary
  colour per pixel, MS Paint "pattern fill" style, plus ordered Bayer
  levels.
- **Version**: a named snapshot of the whole document with a timestamp,
  message and thumbnail.
- **Note**: free text on the document or a frame, or a pin at a pixel
  position on the canvas.
- **Slice**: a named rectangle (hitbox, 9-patch, pivot) exported in the sheet
  metadata.

## Feature catalogue

Ordered by build sequence. Each heading is one `tests/features/<name>.rs`
file.

### 1. Canvas & documents (`documents.rs`)
- Given a new 32×32 document, then it has one layer "Layer 1", one frame of
  100 ms, a transparent cel, and the default palette.
- Given a document, when I resize the canvas to 48×16 anchored top-left, then
  existing pixels keep their coordinates and new area is transparent.
- Given a saved `.spry` file, when I open it, then every layer, frame, cel,
  tag, note and version comes back identical.

### 2. Drawing tools (`drawing.rs`)
- Pencil, eraser, line, rectangle (outline/filled), ellipse (outline/filled),
  bucket fill (contiguous / all matching colour), eyedropper, colour replace,
  spray.
- Given an empty canvas, when I draw a line from (0,0) to (7,3), then exactly
  the Bresenham pixels are painted.
- Given *pixel-perfect* is on, when I draw an L-shaped freehand stroke, then
  no corner "double pixels" remain.
- Given horizontal symmetry, when I paint (2,5) on a 16-wide canvas, then
  (13,5) is painted too.
- Given brush size 3, when I click (5,5), then a 3×3 square centred on (5,5)
  is painted.
- Given a locked layer, when I draw, then nothing changes and the editor says
  why.

### 3. MS Paint-style dithers (`dithering.rs`)
- Pattern library: the classic Paint pattern fills (solid, checker, 25%,
  75%, dots, horizontal, vertical, both diagonals, cross-hatch, bricks: 11 in
  all), plus Bayer 2×2/4×4/8×8 levels 0–64.
- Given the checker pattern with primary black and secondary transparent,
  when I fill a 4×4 rectangle, then pixels where (x+y) is even are black and
  the rest are untouched.
- Patterns are anchored to canvas coordinates, so adjoining strokes tile
  without seams.
- Given the dither brush, when I paint over existing art, then only "on"
  pixels change (or both colours change, if two-colour mode is on).
- Gradient dither: dragging a line across a selection fills it with a
  primary→secondary ordered-dither gradient.

### 4. Selection & clipboard (`selection.rs`)
- Rectangle select, lasso (freehand, even-odd fill plus the traced
  outline; Q), magic wand (contiguous colour), select all / none / invert,
  Shift to add.
- Cut, copy and paste into a floating selection that you move with the mouse
  or arrow keys, then stamp down.
- Flip H/V, rotate 90°, nudge 1 px, and outline the selection with the primary
  colour.
- Rotsprite rotation by any angle (Scale2x ×3, rotate, sample down) for
  pieces up to 512 px: lines stay connected and no new colours appear.
- Given a 12×4 bar rotated 30°, then it covers more rows and uses only its
  own colour, and undo straightens it.

### 5. Layers (`layers.rs`)
- Add, remove, rename, reorder, duplicate, merge down, toggle visibility,
  opacity 0–255, and lock.
- Given two layers with overlapping pixels, when the top is at 50% opacity,
  then the composite is the alpha blend.
- Blend modes: Aseprite's nineteen (normal, multiply, screen, overlay,
  darken, lighten, colour dodge/burn, hard/soft light, difference,
  exclusion, hue, saturation, colour, luminosity, addition, subtract,
  divide), per the W3C compositing formulas.
- Given an orange layer under a 50% grey multiply layer, then the composite
  is (128, 64, 0).
- Cel opacity: each cel fades on its own, on top of its layer's opacity. A
  clone shares pixels but keeps its own opacity.
- Groups: layers can go into groups, at any depth. A group composites its
  children first and lays the result down with its own opacity and blend.
  Hiding a group hides everything in it. Deleting or duplicating a group
  takes everything in it. A group can't hold pixels, and can't go inside
  itself.
- Given a group holding a layer, when I hide the group, then that layer's
  pixels vanish from the composite. When I delete the group and undo, then
  both come back.

### 6. Frames, animation & onion skin (`animation.rs`)
- Add, duplicate, remove and reorder frames, with per-frame duration.
- Tags with loop direction. Playback in the preview honours tag, direction
  and durations.
- Onion skin: previous/next N frames tinted red/blue at a set opacity, shown
  on screen only and never exported.
- Given frames of 100, 200 and 100 ms, when playback runs for 250 ms, then
  frame 2 is showing.
- Given a ping-pong tag on frames 1–3, then the play order is 1,2,3,2,1,2,…

### 7. Clones (`clones.rs`)
- **Linked cels:** "Duplicate frame as clone" links cels. Painting one paints
  all of them. "Unlink" gives the cel its own copy of the pixels.
- Given frame 1 cloned to frame 2, when I paint (3,3) on frame 2, then frame
  1 shows it too.
- Given a linked cel, when I unlink it and paint, then the other frame is
  unchanged.
- **Clone stamp tool:** Alt-click sets the source, and painting copies pixels
  from the source offset, Paint/Photoshop style.

### 8. Palette & colour (`palette.rs`)
- Built-in palettes: PICO-8, DB16, DB32, Endesga 32, Game Boy (4), NES-like
  (54), Sweetie 16.
- Add, remove and edit colours. HSV picker with primary/secondary swap (X).
- Load and save `.hex` and `.gpl` palettes.
- **Ramp shading tool:** Given a ramp [dark, mid, light], when I shade-lighten
  a "mid" pixel, then it becomes "light".
- **Palette swap:** remap one colour to another across the whole document.
- **Extract palette:** build a palette from the image's unique colours.

### 9. History (`history.rs`)
- Every command can be undone and redone. A whole stroke undoes as one step.
- Given 3 strokes, when I undo twice and redo once, then exactly strokes 1
  and 2 are visible.

### 10. Versioning (`versioning.rs`)
- "Commit version" with a message stores a snapshot, timestamp and thumbnail
  inside the document.
- Auto-version on every save, deduplicated: identical content doesn't make a
  new version.
- Browse versions in a timeline, preview one, restore it (restoring is
  itself undoable), or branch it into a new document.
- Given versions v1 "base" and v2 "armour", when I restore v1, then the
  canvas equals v1 and v2 is still in the list.
- Visual diff: changed pixels between two versions are highlighted.

### 11. Notes (`notes.rs`)
- A document note (markdown text), per-frame notes, and canvas pins (a note
  at a pixel, drawn as a small glass marker).
- Notes survive save/load and are exported in the sheet JSON (`meta.notes`).
- An LLM can read and write notes over MCP (art direction, TODOs for a
  human).

### 12. Exports (`exports.rs`)
- Current frame to PNG at a scale of 1–16×, nearest neighbour.
- Animation to GIF honouring durations and the tag range, transparency
  preserved.
- PNG sequence (`name_000.png`, …).
- Animated PNG (APNG) and animated lossless WebP: full alpha and exact
  millisecond delays. One frame as lossless WebP.
- Given 15 ms frames, when I export an APNG, then each frame decodes with
  its pixels and exactly 15 ms.
- Given a 3-frame 8×8 animation, when I export a GIF, then decoding it yields
  3 frames with the same pixels and delays.

### 13. Automatic sprite sheets (`sprite_sheets.rs`)
- Layouts: horizontal strip, vertical strip, grid (auto-square or fixed
  columns), and packed (shelf packing by height, best with trimming).
- Options: padding, border, trim transparent edges, merge duplicate frames,
  and one tag or all.
- Writes `sheet.png` and `sheet.json` in Aseprite hash format (`frames`,
  `meta.frameTags`, `meta.slices`, `meta.size`, `meta.notes`).
- Given 4 frames where frames 2 and 4 are identical, when I pack with
  "merge duplicates", then the sheet holds 3 images and frames 2 and 4 point
  to the same rect.
- **Import sheet:** slice a PNG into frames by a grid size, or auto-detect the
  grid from transparent gutters.

### 14. Previews (`previews.rs`)
- A floating glass preview panel showing the animation at real size and 2×,
  with play/pause and a tag selector.
- Tiled preview (3×3 wrap) for making seamless textures.
- A live thumbnail in the layers panel, frame thumbnails in the timeline, and
  version thumbnails.

### 15. MCP server (`mcp.rs`)
- Resources: `spry://document` (the summary) and `spry://frames/{n}.png`
  (each frame flattened at its real size), listed and read through
  `resources/list`, `resources/templates/list` and `resources/read`.
- `spry mcp [file.spry]` speaks MCP (JSON-RPC 2.0, protocol 2025-06-18) over
  stdio.
- The running app serves the same tools over HTTP at `127.0.0.1:7477/mcp`
  when you enable it in settings, so an LLM can paint while you watch.
- Tools: `document_info`, `new_document`, `open_document`, `save_document`,
  `set_pixels`, `draw_line`, `draw_rect`, `draw_ellipse`, `fill`,
  `dither_fill`, `get_pixels`, `render_png` (returns an MCP image), `add_layer`,
  `add_frame`, `clone_frame`, `set_palette`, `add_tag`, `add_note`,
  `list_notes`, `commit_version`, `list_versions`, `restore_version`,
  `export_png`, `export_gif`, `export_sheet`, `undo`, `redo`.
- Given the `spry mcp` binary, when a client sends `initialize` then
  `tools/list`, then every tool above is listed with a JSON schema.
- Given a session, when the client calls `draw_rect` then `render_png`, then
  the returned PNG shows the rectangle.

### 16. AI: LLM pixel agent (`ai_agent.rs`)
- A chat panel where you describe a change ("add a 1px dark outline", "make
  a 4-frame idle bob"). The model (Claude by default, or Ollama locally) calls
  the same tool set as MCP, and each turn is one undo group.
- Before each turn the agent gets a rendered PNG of the canvas, so it can
  see what it's doing.
- Given a scripted fake LLM server that answers with a `draw_rect` tool call,
  when I send a prompt, then the rectangle appears and one undo removes it.

### 17. AI: diffusion with micro pixel-art models (`ai_diffusion.rs`)
- Backends: Automatic1111/Forge (`/sdapi/v1/txt2img`, `/img2img`) and
  ComfyUI (`/prompt` + `/history` + `/view`) with an editable workflow
  template.
- Presets tuned for small pixel models and LoRAs (prompt suffixes, low step
  counts, small native sizes).
- **Pixelise pipeline:** downscale to the canvas size by dominant colour per
  block → optional background removal (flood from the corners) → quantise to
  the document palette (nearest in Oklab) with optional ordered dither. The
  result lands as a new layer.
- img2img from the current frame ("restyle this sprite").
- Given a fake A1111 server returning a 256×256 two-colour image, when I
  generate into a 32×32 document with the PICO-8 palette, then a new layer
  appears whose pixels are all PICO-8 colours.

### 18. Glass UI (`ui` module; scenario-tested through `editor` commands)
- A soft gradient backdrop. Translucent rounded panels with hairline light
  borders and soft shadows. A floating tool dock on the left, a floating
  timeline at the bottom, inspector panels on the right, and a dark/light
  theme.
- Command palette (Ctrl+K) lists every action by name, so every action can be
  found and run from the keyboard.
- Aseprite-style shortcuts: B pencil, E eraser, G bucket, I eyedropper, L
  line, U rect/ellipse, M select, V move, X swap colours, Alt+[ / Alt+] for
  frames, Ctrl+Z/Y, Enter play.
- Zoom (wheel, 1–64×) about the cursor, pan (space-drag / middle mouse),
  pixel grid at ≥ 6×, custom tile grid.

### 19. Innovation shelf
Built after the core, each with its own scenarios when it starts:
- **Auto outline / drop shadow** as one-click layer effects (implemented as
  real commands, not filters on export).
- **Tween frames**: generate in-betweens by moving a selection along a path.
- **Palette from prompt**: the LLM proposes a palette for a mood.
- **Animation easing preview**: timeline shows the timing curve.

### 20. Opening Aseprite files (`aseprite_import.rs`)
Most pixel artists' existing work is in `.aseprite`/`.ase` files, so Spry
opens them (the published file spec) as new, unsaved documents.
- Imported as they are: canvas size, frames and their durations, layers
  (name, visibility, lock, opacity, blend mode), groups at any depth, cels
  at their offsets (raw, compressed and linked, so clones stay clones) with
  their own opacity, tags with direction, the palette, slices, and RGBA,
  grayscale and indexed colour modes.
- Changed on the way in, and named in the import report: tilemap layers
  are drawn out tile by tile (with flips) into ordinary image layers, since
  Spry has no tilesets, and slices keep only their first key.
- Given a tilemap layer, when I open the file, then each tile's pixels sit
  at its grid position, flipped as the map says.
- Given an Aseprite file with two layers, a cel offset by (2, 3) and a
  linked cel, when I open it, then the pixels sit at the offset, the linked
  frames share one image, and the layer names match.
- Given an indexed-colour file, when I open it, then the transparent index
  becomes transparent and every other index takes its palette colour.
- Given a truncated or corrupt file, when I open it, then I get an error
  saying so, never a crash.
