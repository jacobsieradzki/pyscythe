//! Acceptance tests for `pyscythe cycles`.

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
fn reports_runtime_import_cycles_but_not_type_only_or_deferred_ones() {
    let output = pyscythe()
        .args(["cycles", "--format", "json"])
        .arg(fixture("cycles"))
        .output()
        .expect("runs");

    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(report["kind"], "cycles");
    let findings = report["findings"].as_array().expect("findings array");
    assert_eq!(findings.len(), 1, "{report}");
    assert_eq!(findings[0]["rule"], "circular-import");
    assert_eq!(
        findings[0]["message"],
        "import cycle: pkg.a -> pkg.b -> pkg.a"
    );
    let chain = findings[0]["chain"].as_array().expect("chain");
    assert_eq!(chain.len(), 2);
    assert_eq!(chain[0]["position"]["line"], 1);
}

#[test]
fn human_output_names_the_cycle() {
    pyscythe()
        .arg("cycles")
        .arg(fixture("cycles"))
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "a.py:1:1  import cycle: pkg.a -> pkg.b -> pkg.a",
        ))
        .stdout(predicate::str::contains("1 finding(s) in 6 files."));
}

#[test]
fn a_project_without_cycles_is_clean() {
    pyscythe()
        .arg("cycles")
        .arg(fixture("simple_unused"))
        .assert()
        .success()
        .stdout(predicate::str::contains("No import cycles found"));
}
