# Spry: Rules of the House

These are working rules, not suggestions. They follow the opinionated
consultancy tradition (Pivotal Labs, thoughtbot): test first, small steps,
clear names, nothing half-built in `main`. When a rule gets in the way, change
the rule in this file in its own commit and say why. Don't quietly break it.

The product itself (what Spry is, who it's for, every feature and its
acceptance scenarios) lives in [`docs/BIBLE.md`](docs/BIBLE.md). Read it
before you add a feature. If a feature isn't there, add it there first.

## 1. The original brief

Kept word for word, typos included, because it's the source of truth:

> using rust or go, build a cross platform sprite and animation and pixel art
> editor that supports mspaint style dithers, llm mcp, llm static diffusion
> with micro pixel art models, clones, aseprite features, an intuivie modern
> apple glass like ui, previews, exports, automatic sprite sheeting,
> versioning, notes, and any other useful and innovative features.

And the standing order on how to build it:

> use pivotal thoughtbot style opoinionated consultant, high test coverage,
> bdd e2e style, no placeholder code, only working code, literate and
> expressive code, write it in rules and bibles along with my roiginal prompt.

## 2. Non-negotiables

1. **Only working code.** No `todo!()`, `unimplemented!()`, `// TODO`, stub
   functions, buttons that do nothing, or "coming soon" menu items. If a
   feature isn't finished it isn't merged or shown. A smaller real feature
   beats a bigger fake one.
2. **Red, green, refactor.** Write the failing scenario or test first, make it
   pass the simplest honest way, then clean up. Every bug fix starts with a
   test that reproduces the bug.
3. **Behaviour before units.** Each feature in the Bible has Given/When/Then
   scenarios. They live in `tests/features/` and run against the real
   headless editor, the real file formats and the real MCP binary. Unit tests
   cover the algorithms underneath: rasterising, dithering, packing,
   quantising.
4. **High coverage, measured.** Aim for 90%+ line coverage of `src/` outside
   the egui view layer (`cargo llvm-cov`). The view layer stays thin so
   that's easy: views read state and send `Command`s, nothing more.
5. **Green on every commit.** `cargo fmt --check`, `cargo clippy --all-targets
   -- -D warnings` and `cargo test` all pass before anything is committed.
6. **No fake infrastructure in tests.** Use a real local HTTP server (via
   `tiny_http`) to stand in for diffusion and LLM backends, real temp
   directories for files, and the real compiled binary for MCP. Don't mock
   our own modules.

## 3. Architecture in one breath

`editor` is the pure, headless editor: a `Document` (canvas, layers, frames, cels,
tags, palette, notes, version history) changed only through `Command`s that
the `History` can undo. Everything else is an adapter around `editor`:

- `ui` is the egui/eframe "glass" desktop app. It draws state and turns input
  into commands.
- `mcp` serves the same commands to any LLM as MCP tools, over stdio
  (`spry mcp`) or local HTTP from the running app.
- `ai` talks to diffusion backends (Automatic1111/Forge, ComfyUI) and chat
  LLMs (Claude, Ollama), then pixelises and quantises results into the
  document.
- `io` reads and writes `.spry` files and exports PNG, GIF, PNG sequences and
  sprite sheets with Aseprite-compatible JSON.

Dependencies point inward. `editor` never imports egui, HTTP or the filesystem.
(It isn't called `core` so it can't shadow Rust's own `core` crate.)

## 4. Code style: literate and expressive

- **Names tell the story.** `flood_fill_contiguous`, not `ff`.
  `Document::linked_cels_of`, not `get_links`. Domain words come from the
  Bible's glossary (cel, frame, tag, clone, ramp, dither pattern) and mean
  exactly what it says.
- **Small functions, one level of abstraction each.** If a function needs a
  comment to split it into sections, split it into functions.
- **Comments say why, not what.** Each module opens with a short `//!`
  paragraph saying what it's for and how it fits in. Comment only the
  surprising parts, like the reason for a pixel-perfect rule or a format quirk.
- **Make illegal states unrepresentable.** Use enums over flags and newtypes
  over bare `usize` where indexes are easy to mix up (`LayerId`, `FrameIndex`).
- **Errors are values.** Return `Result` with context (`anyhow::Context`) at
  the edges and typed errors inside `editor`. No `unwrap()` outside tests,
  except on invariants that are proven right there and commented.
- **Tests read like specs.** Name them as sentences:
  `fn bucket_fill_stops_at_a_different_colour()`. Scenarios use the
  `given_/when_/then_` helpers in `tests/support/`.

## 5. Workflow

- Small commits, one idea each, present tense: "Add ordered Bayer dither
  brush". End every commit message with the `Co-Authored-By` trailer.
- Do features in the Bible's order. Each one is Bible scenario → failing
  test → code → refactor → commit.
- Keep `README.md` honest. It lists only what works today.

## 6. This machine

This is a shared desktop that drives video all day. Run builds and tests
inside a capped scope, and never let a job take the whole box:

```sh
systemd-run --user --scope -p MemoryMax=6G -p MemorySwapMax=1G -p CPUQuota=600% -p IOWeight=20 nice -n 15 cargo test
```

If a job hits the cap, shrink the job (`-j4`, fewer test threads), not the
cap. Network calls go through OpenSnitch. If one stalls, retry it before
suspecting anything else.
