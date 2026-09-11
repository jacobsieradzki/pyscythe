//! Acceptance tests for `pyscythe dupes`.

use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn pyscythe() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pyscythe"))
}

#[test]
fn renamed_copies_are_found_in_mild_mode() {
    let output = pyscythe()
        .args(["dupes", "--format", "json"])
        .arg(fixture("dupes"))
        .output()
        .expect("runs");

    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(report["kind"], "dupes");
    let findings = report["findings"].as_array().expect("findings");
    assert_eq!(findings.len(), 1, "{report}");
    let finding = &findings[0];
    assert_eq!(finding["rule"], "duplicate-code");
    assert_eq!(finding["module"], "pkg.first");
    assert_eq!(finding["position"]["line"], 1);
    assert!(finding["lines"].as_u64().expect("lines") >= 13, "{report}");
    assert_eq!(finding["other"]["module"], "pkg.second");
    assert_eq!(finding["other"]["position"]["line"], 1);
    let duplication = &report["summary"]["duplication"];
    assert_eq!(duplication["clones"], 1);
    assert!(duplication["percent_tenths"].as_u64().expect("percent") > 500);
}

#[test]
fn strict_mode_does_not_match_renamed_copies() {
    pyscythe()
        .args(["dupes", "--mode", "strict"])
        .arg(fixture("dupes"))
        .assert()
        .success()
        .stdout(predicate::str::contains("No duplicated code found"));
}

#[test]
fn thresholds_are_adjustable() {
    pyscythe()
        .args(["dupes", "--min-lines", "40"])
        .arg(fixture("dupes"))
        .assert()
        .success();
}

#[test]
fn human_output_names_both_locations() {
    pyscythe()
        .arg("dupes")
        .arg(fixture("dupes"))
        .assert()
        .code(1)
        .stdout(predicate::str::contains("first.py:1:1  "))
        .stdout(predicate::str::contains("duplicated at pkg.second:1"))
        .stdout(predicate::str::contains("% duplicated:"));
}
