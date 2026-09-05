//! Acceptance tests: run the real binary against fixture projects.

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
fn reports_unused_module_level_definitions_as_json() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("simple_unused"))
        .output()
        .expect("runs");

    assert_eq!(output.status.code(), Some(1), "findings exit with 1");

    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(report["kind"], "dead-code");
    assert_eq!(report["schema_version"], 1);

    let mut symbols: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .map(|f| f["symbol"].as_str().expect("symbol name"))
        .collect();
    symbols.sort_unstable();
    assert_eq!(symbols, ["UnusedThing", "unused_helper"]);

    let unused_helper = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .find(|f| f["symbol"] == "unused_helper")
        .expect("unused_helper finding");
    assert_eq!(unused_helper["rule"], "unused-function");
    assert_eq!(unused_helper["module"], "pkg.helpers");
    assert_eq!(unused_helper["position"]["line"], 5);
    assert_eq!(unused_helper["confidence"], "medium");
}

#[test]
fn human_format_lists_each_finding_with_its_location() {
    pyscythe()
        .arg("dead-code")
        .arg(fixture("simple_unused"))
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "helpers.py:5:5  function `unused_helper` is never used",
        ))
        .stdout(predicate::str::contains(
            "helpers.py:13:7  class `UnusedThing` is never used",
        ))
        .stdout(predicate::str::contains("2 finding(s)"));
}

#[test]
fn a_clean_project_exits_zero() {
    pyscythe()
        .arg("dead-code")
        .arg(fixture("all_used"))
        .assert()
        .success()
        .stdout(predicate::str::contains("No dead code found"));
}

#[test]
fn a_relative_path_is_resolved_against_the_working_directory() {
    pyscythe()
        .current_dir(fixture("simple_unused"))
        .args(["dead-code", "."])
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "function `unused_helper` is never used",
        ));
}

#[test]
fn a_missing_path_is_an_error() {
    pyscythe()
        .arg("dead-code")
        .arg(fixture("does_not_exist"))
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error:"));
}
