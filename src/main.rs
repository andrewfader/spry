//! `spry`: see `spry --help`.

use spry::cli::{self, Invocation};
use spry::studio::Studio;
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let invocation = match cli::parse(&arguments) {
        Ok(invocation) => invocation,
        Err(error) => {
            eprintln!("spry: {error:#}");
            return ExitCode::from(2);
        }
    };
    let outcome = match invocation {
        Invocation::Gui(file) => spry::ui::run(file, None).map_err(|e| anyhow::anyhow!("{e}")),
        Invocation::Capture { out, file } => spry::ui::run(file, Some(out)).map_err(|e| anyhow::anyhow!("{e}")),
        Invocation::Mcp(file) => serve_mcp(file),
        headless => cli::run_headless(headless).map(|output| println!("{output}")),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("spry: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn serve_mcp(file: Option<std::path::PathBuf>) -> anyhow::Result<()> {
    let studio = match file {
        Some(path) if path.exists() => Studio::open(&path)?,
        Some(path) => {
            // A path that doesn't exist yet is where the agent will save.
            let mut studio = Studio::new(32, 32);
            studio.path = Some(path);
            studio
        }
        None => Studio::new(32, 32),
    };
    spry::mcp::serve_stdio(studio)?;
    Ok(())
}
