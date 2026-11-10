// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `ooconv`: convert between Office Open XML and OpenDocument.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ooconv::{Error, Format, Options};

const USAGE: &str = "\
usage: ooconv <input> [<output>] [--to FORMAT] [--strict] [--quiet]
       ooconv info <input>

  Converts xlsx <-> ods/fods and docx <-> odt/fodt. The input's format is read from its bytes;
  the output's from --to, else the output's extension, else the input's counterpart.

  --to FORMAT   xlsx, ods, fods, docx, odt or fodt
  --strict      refuse (exit 3) rather than write a document missing anything
  --quiet       do not list what the conversion could not carry

exit status: 0 converted, 1 usage or I/O error, 2 unreadable input, 3 refused under --strict";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("ooconv: {message}");
            ExitCode::from(1)
        }
    }
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let mut positional = Vec::new();
    let mut to = None;
    let mut options = Options::default();
    let mut quiet = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(ExitCode::SUCCESS);
            }
            "--version" => {
                println!("ooconv {}", env!("CARGO_PKG_VERSION"));
                return Ok(ExitCode::SUCCESS);
            }
            "--strict" => options.strict = true,
            "--quiet" | "-q" => quiet = true,
            "--to" => {
                let value = iter.next().ok_or("--to needs a format")?;
                to = Some(Format::from_extension(value).ok_or(format!("unknown format {value}"))?);
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown option {other}\n{USAGE}"));
            }
            other => positional.push(PathBuf::from(other)),
        }
    }
    if positional.first().is_some_and(|p| p == Path::new("info")) && positional.len() == 2 {
        return info(&positional[1]);
    }
    let (input, output) = match positional.as_slice() {
        [input] => (input.clone(), None),
        [input, output] => (input.clone(), Some(output.clone())),
        _ => return Err(USAGE.to_owned()),
    };
    let bytes = std::fs::read(&input).map_err(|e| format!("{}: {e}", input.display()))?;
    let from = match ooconv::sniff(&bytes) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("ooconv: {}: {e}", input.display());
            return Ok(ExitCode::from(2));
        }
    };
    let to = to
        .or_else(|| {
            output
                .as_ref()
                .and_then(|o| o.extension())
                .and_then(|e| e.to_str())
                .and_then(Format::from_extension)
        })
        .unwrap_or(from.counterpart());
    let output = output.unwrap_or_else(|| input.with_extension(to.extension()));
    if output == input {
        return Err("refusing to overwrite the input".to_owned());
    }
    match ooconv::convert_with(&bytes, to, options) {
        Ok(converted) => {
            std::fs::write(&output, &converted.bytes)
                .map_err(|e| format!("{}: {e}", output.display()))?;
            if !quiet {
                for loss in &converted.report.losses {
                    eprintln!("not carried: {loss}");
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(Error::WouldLose(report)) => {
            for loss in &report.losses {
                eprintln!("would lose: {loss}");
            }
            eprintln!("ooconv: refused under --strict; nothing written");
            Ok(ExitCode::from(3))
        }
        Err(e @ Error::KindMismatch { .. }) => Err(e.to_string()),
        Err(e) => {
            eprintln!("ooconv: {}: {e}", input.display());
            Ok(ExitCode::from(2))
        }
    }
}

fn info(input: &Path) -> Result<ExitCode, String> {
    let bytes = std::fs::read(input).map_err(|e| format!("{}: {e}", input.display()))?;
    match ooconv::sniff(&bytes) {
        Ok(format) => {
            println!("{} ({})", format.extension(), format.kind());
            Ok(ExitCode::SUCCESS)
        }
        Err(e) => {
            eprintln!("ooconv: {}: {e}", input.display());
            Ok(ExitCode::from(2))
        }
    }
}
