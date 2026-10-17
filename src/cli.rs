//! The command line: open the app, serve MCP, or export without a window.
//!
//! Headless export makes Spry usable in build pipelines. A game repo can
//! keep `.spry` sources and generate sheets in CI.

use crate::io::{self, SheetLayout, SheetOptions};
use crate::studio::Studio;
use anyhow::{bail, Context, Result};
use std::path::PathBuf;

pub const USAGE: &str = "\
Spry: pixel art, sprites and animation

USAGE:
  spry [FILE]                     open the editor (optionally with a .spry/.png/.gif)
  spry mcp [FILE]                 serve MCP over stdio for LLM agents
  spry info FILE                  describe a sprite
  spry export FILE [OPTIONS]      export without opening a window
  spry capture OUT.png [FILE]     open the window, save a picture of it, close

EXPORT OPTIONS:
  --png OUT        one frame as PNG (see --frame)
  --gif OUT        animated GIF (see --tag)
  --apng OUT       animated PNG: full alpha, exact delays (see --tag)
  --webp OUT       one frame as lossless WebP (see --frame)
  --webp-anim OUT  animated lossless WebP: full alpha, exact delays (see --tag)
  --sheet OUT      sprite sheet PNG plus Aseprite JSON beside it
  --frame N        frame to export with --png or --webp (default 0)
  --tag NAME       only this tag's frames (--gif, --apng, --sheet)
  --scale N        nearest-neighbour scale (default 1)
  --layout L       horizontal | vertical | grid | packed (default grid)
  --columns N      fixed grid columns
  --padding N      pixels between sheet cells
  --trim           trim transparent edges in the sheet
  --merge          merge identical frames in the sheet

  -h, --help       show this help
  -V, --version    show the version
";

#[derive(Debug, PartialEq)]
pub enum Invocation {
    Gui(Option<PathBuf>),
    /// Open the window on `file`, save what it shows to `out`, and quit:
    /// for documentation, bug reports, and checking the look of the UI.
    Capture {
        out: PathBuf,
        file: Option<PathBuf>,
    },
    Mcp(Option<PathBuf>),
    Info(PathBuf),
    Export {
        source: PathBuf,
        target: ExportTarget,
        options: ExportOptions,
    },
    Help,
    Version,
}

#[derive(Debug, PartialEq)]
pub enum ExportTarget {
    Png(PathBuf),
    Gif(PathBuf),
    Apng(PathBuf),
    Webp(PathBuf),
    AnimatedWebp(PathBuf),
    Sheet(PathBuf),
}

#[derive(Debug, PartialEq, Default)]
pub struct ExportOptions {
    pub frame: usize,
    pub tag: Option<String>,
    pub scale: u32,
    pub sheet: SheetOptions,
}

pub fn parse(arguments: &[String]) -> Result<Invocation> {
    let mut words = arguments.iter().map(String::as_str);
    match words.next() {
        None => Ok(Invocation::Gui(None)),
        Some("-h" | "--help" | "help") => Ok(Invocation::Help),
        Some("-V" | "--version") => Ok(Invocation::Version),
        Some("mcp") => Ok(Invocation::Mcp(words.next().map(PathBuf::from))),
        Some("info") => Ok(Invocation::Info(PathBuf::from(words.next().context("`spry info` needs a file")?))),
        Some("export") => parse_export(words.collect()),
        Some("capture") => {
            let out = PathBuf::from(words.next().context("`spry capture` needs a .png to write")?);
            Ok(Invocation::Capture { out, file: words.next().map(PathBuf::from) })
        }
        Some(flag) if flag.starts_with('-') => bail!("unknown option `{flag}`\n\n{USAGE}"),
        Some(path) => Ok(Invocation::Gui(Some(PathBuf::from(path)))),
    }
}

fn parse_export(words: Vec<&str>) -> Result<Invocation> {
    let mut words = words.into_iter();
    let source = PathBuf::from(words.next().context("`spry export` needs a source file")?);
    let mut target = None;
    let mut options = ExportOptions { scale: 1, ..ExportOptions::default() };
    let mut layout = "grid".to_string();
    let mut columns = None;
    while let Some(flag) = words.next() {
        let mut value = || words.next().with_context(|| format!("`{flag}` needs a value"));
        let number =
            |text: &str| text.parse::<u32>().with_context(|| format!("`{flag}` expects a whole number, not `{text}`"));
        match flag {
            "--png" => target = Some(ExportTarget::Png(value()?.into())),
            "--gif" => target = Some(ExportTarget::Gif(value()?.into())),
            "--apng" => target = Some(ExportTarget::Apng(value()?.into())),
            "--webp" => target = Some(ExportTarget::Webp(value()?.into())),
            "--webp-anim" => target = Some(ExportTarget::AnimatedWebp(value()?.into())),
            "--sheet" => target = Some(ExportTarget::Sheet(value()?.into())),
            "--frame" => options.frame = number(value()?)? as usize,
            "--tag" => options.tag = Some(value()?.to_string()),
            "--scale" => options.scale = number(value()?)?.clamp(1, 64),
            "--layout" => layout = value()?.to_string(),
            "--columns" => columns = Some(number(value()?)?.max(1)),
            "--padding" => options.sheet.padding = number(value()?)?,
            "--trim" => options.sheet.trim = true,
            "--merge" => options.sheet.merge_duplicates = true,
            other => bail!("unknown export option `{other}`\n\n{USAGE}"),
        }
    }
    options.sheet.layout = match layout.as_str() {
        "horizontal" => SheetLayout::Horizontal,
        "vertical" => SheetLayout::Vertical,
        "grid" => SheetLayout::Grid { columns },
        "packed" => SheetLayout::Packed,
        other => bail!("unknown layout `{other}`; use horizontal, vertical, grid or packed"),
    };
    options.sheet.tag = options.tag.clone();
    options.sheet.scale = options.scale;
    let target = target.context("say what to export: --png, --gif, --apng, --webp, --webp-anim or --sheet")?;
    Ok(Invocation::Export { source, target, options })
}

/// Runs a headless invocation and returns what to print.
pub fn run_headless(invocation: Invocation) -> Result<String> {
    match invocation {
        Invocation::Help => Ok(USAGE.to_string()),
        Invocation::Version => Ok(format!("spry {}", env!("CARGO_PKG_VERSION"))),
        Invocation::Info(path) => {
            let mut studio = Studio::open(&path)?;
            let info = crate::mcp::tools::call(&mut studio, "document_info", &serde_json::json!({}))
                .map_err(anyhow::Error::msg)?;
            Ok(info.text)
        }
        Invocation::Export { source, target, options } => {
            let studio = Studio::open(&source)?;
            let sprite = studio.editor.sprite();
            match target {
                ExportTarget::Png(out) => {
                    if options.frame >= sprite.frame_count() {
                        bail!("there is no frame {} (the sprite has {})", options.frame, sprite.frame_count());
                    }
                    io::export_png(&out, sprite, options.frame, options.scale)?;
                    Ok(format!("wrote {}", out.display()))
                }
                ExportTarget::Gif(out) => {
                    io::export_gif(&out, sprite, options.tag.as_deref(), options.scale)?;
                    Ok(format!("wrote {}", out.display()))
                }
                ExportTarget::Apng(out) => {
                    io::export_apng(&out, sprite, options.tag.as_deref(), options.scale)?;
                    Ok(format!("wrote {}", out.display()))
                }
                ExportTarget::AnimatedWebp(out) => {
                    io::export_animated_webp(&out, sprite, options.tag.as_deref(), options.scale)?;
                    Ok(format!("wrote {}", out.display()))
                }
                ExportTarget::Webp(out) => {
                    if options.frame >= sprite.frame_count() {
                        bail!("there is no frame {} (the sprite has {})", options.frame, sprite.frame_count());
                    }
                    io::export_webp(&out, sprite, options.frame, options.scale)?;
                    Ok(format!("wrote {}", out.display()))
                }
                ExportTarget::Sheet(out) => {
                    let sheet = io::export_sheet(&out, sprite, &studio.title(), &options.sheet)?;
                    Ok(format!(
                        "wrote {} ({}×{}, {} frames) and {}",
                        out.display(),
                        sheet.image.width(),
                        sheet.image.height(),
                        sheet.frames.len(),
                        out.with_extension("json").display()
                    ))
                }
            }
        }
        Invocation::Gui(_) | Invocation::Capture { .. } | Invocation::Mcp(_) => {
            bail!("this invocation needs a window or a stdio session")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn no_arguments_opens_the_editor() {
        assert_eq!(parse(&[]).unwrap(), Invocation::Gui(None));
        assert_eq!(parse(&args("hero.spry")).unwrap(), Invocation::Gui(Some("hero.spry".into())));
        assert_eq!(
            parse(&args("capture shot.png hero.spry")).unwrap(),
            Invocation::Capture { out: "shot.png".into(), file: Some("hero.spry".into()) }
        );
        assert!(parse(&args("capture")).unwrap_err().to_string().contains("needs a .png"));
    }

    #[test]
    fn mcp_takes_an_optional_file() {
        assert_eq!(parse(&args("mcp")).unwrap(), Invocation::Mcp(None));
        assert_eq!(parse(&args("mcp a.spry")).unwrap(), Invocation::Mcp(Some("a.spry".into())));
    }

    #[test]
    fn export_reads_its_options() {
        let Invocation::Export { source, target, options } =
            parse(&args("export a.spry --sheet out.png --layout packed --trim --merge --scale 2 --tag walk")).unwrap()
        else {
            panic!("expected an export");
        };
        assert_eq!(source, PathBuf::from("a.spry"));
        assert_eq!(target, ExportTarget::Sheet("out.png".into()));
        assert_eq!(options.sheet.layout, SheetLayout::Packed);
        assert!(options.sheet.trim && options.sheet.merge_duplicates);
        assert_eq!((options.scale, options.sheet.scale), (2, 2));
        assert_eq!(options.sheet.tag.as_deref(), Some("walk"));
    }

    #[test]
    fn export_mistakes_are_explained() {
        assert!(parse(&args("export a.spry")).unwrap_err().to_string().contains("--png"));
        assert!(parse(&args("export a.spry --png")).unwrap_err().to_string().contains("needs a value"));
        assert!(parse(&args("export a.spry --png x.png --scale big"))
            .unwrap_err()
            .to_string()
            .contains("whole number"));
        assert!(parse(&args("--bogus")).is_err());
    }
}
