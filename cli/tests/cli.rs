// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The `ooconv` binary: each documented exit status, `--strict`, and output named for the
//! format asked for.

use std::path::{Path, PathBuf};
use std::process::Command;

fn ooconv(args: &[&Path]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ooconv"))
        .args(args)
        .output()
        .expect("runs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn corpus(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ooconv/tests/data/corpus")
        .join(rel)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ooconv-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn converts_to_the_format_the_output_names() {
    let dir = scratch("convert");
    for (input, output) in [
        ("ods/fictional_monthly_budget.ods", "budget.xlsx"),
        ("xlsx/values/numbers.xlsx", "numbers.fods"),
        ("docx/sample.docx", "sample.odt"),
        ("odt/01-text-and-formatting.odt", "text.docx"),
    ] {
        let out = dir.join(output);
        let (code, err) = ooconv(&[&corpus(input), &out]);
        assert_eq!(code, 0, "{input}: {err}");
        let bytes = std::fs::read(&out).unwrap();
        let ext = out.extension().unwrap().to_str().unwrap();
        assert_eq!(ooconv::sniff(&bytes).unwrap().extension(), ext);
    }
}

#[test]
fn strict_refuses_and_writes_nothing() {
    let dir = scratch("strict");
    let out = dir.join("out.xlsx");
    // Defined names are a reported loss.
    let input = corpus("xlsx/document/defined-names.xlsx");
    let (code, err) = ooconv(&[&input, &out, Path::new("--strict")]);
    assert_eq!(code, 3, "{err}");
    assert!(err.contains("defined name"), "{err}");
    assert!(!out.exists());
}

#[test]
fn unreadable_input_is_status_2_and_a_kind_mismatch_status_1() {
    let dir = scratch("errors");
    let (code, _) = ooconv(&[&corpus("xlsx/hostile/not-a-zip.xlsx"), &dir.join("x.ods")]);
    assert_eq!(code, 2);
    let (code, err) = ooconv(&[&corpus("docx/sample.docx"), &dir.join("x.ods")]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("text document"), "{err}");
}
