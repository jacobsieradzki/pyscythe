//! Acceptance tests for `pyscythe health`.

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
fn reports_hotspots_with_metrics_and_a_score() {
    let output = pyscythe()
        .args(["health", "--format", "json"])
        .arg(fixture("health"))
        .output()
        .expect("runs");

    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(report["kind"], "health");
    let findings = report["findings"].as_array().expect("findings");
    assert_eq!(findings.len(), 1, "{report}");
    assert_eq!(findings[0]["rule"], "complex-function");
    assert_eq!(findings[0]["function"], "classify");
    assert!(findings[0]["cyclomatic"].as_u64().expect("cyclomatic") > 10);
    assert!(findings[0]["cognitive"].as_u64().expect("cognitive") > 15);
    assert_eq!(findings[0]["parameters"], 6);
    let health = &report["summary"]["health"];
    assert_eq!(health["functions"], 2);
    assert!(health["score"].as_u64().expect("score") < 100);
    assert!(health["grade"].is_string());
}

#[test]
fn human_output_ends_with_the_score() {
    pyscythe()
        .arg("health")
        .arg(fixture("health"))
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "function `classify` has cyclomatic complexity",
        ))
        .stdout(
            predicate::str::is_match(r"Health score \d+/100 \([A-F]\) across 3 files")
                .expect("regex"),
        )
        .stdout(predicate::str::contains("1 hotspot(s)"));
}

#[test]
fn a_simple_project_scores_one_hundred() {
    pyscythe()
        .args(["health"])
        .arg(fixture("all_used"))
        .assert()
        .success()
        .stdout(predicate::str::contains("Health score 100/100 (A)"));
}
