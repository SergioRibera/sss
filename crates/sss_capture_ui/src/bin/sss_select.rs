//! `sss-select` — minimal slurp-equivalent that prints `x,y WxH` to stdout.

use std::path::PathBuf;
use std::process::ExitCode;

use sss_capture_ui::{Outcome, SelectorBuilder, SelectorMode};

fn main() -> ExitCode {
    let mut mode = SelectorMode::Area;
    let mut save: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--monitor" | "-m" => mode = SelectorMode::Monitor,
            "--window" | "-w" => mode = SelectorMode::Window,
            "--area" | "-a" => mode = SelectorMode::Area,
            "--save" | "-s" => {
                save = args.next().map(PathBuf::from);
            }
            "-h" | "--help" => {
                tracing::debug!("usage: sss-select [--area|--monitor|--window] [--save out.png]");
                return ExitCode::SUCCESS;
            }
            other => {
                tracing::error!("sss-select: unknown argument {other:?}");
                return ExitCode::from(2);
            }
        }
    }

    let sel = match SelectorBuilder::default()
        .mode(mode)
        .with_toolbar(false)
        .build()
    {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("sss-select: {e}");
            return ExitCode::FAILURE;
        }
    };

    let result = match sel.run() {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("sss-select: {e}");
            return ExitCode::FAILURE;
        }
    };

    let rect = match &result.outcome {
        Outcome::Region { rect, .. }
        | Outcome::Monitor { rect, .. }
        | Outcome::Window { rect, .. } => *rect,
        Outcome::Cancelled => {
            return ExitCode::from(1);
        }
    };
    println!("{rect}");

    if let Some(path) = save {
        let image = match &result.outcome {
            Outcome::Region { image, .. }
            | Outcome::Monitor { image, .. }
            | Outcome::Window { image, .. } => image.clone(),
            Outcome::Cancelled => None,
        };
        if let Some(img) = image {
            if let Err(e) = img.save(&path) {
                tracing::error!("sss-select: saving {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        }
    }

    ExitCode::SUCCESS
}
