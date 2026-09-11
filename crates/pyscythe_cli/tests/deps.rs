//! Acceptance tests for `pyscythe deps`.

use std::path::PathBuf;

use assert_cmd::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn pyscythe() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pyscythe"))
}

#[test]
fn reports_unused_declared_dependencies_and_unresolved_imports_without_an_environment() {
    let output = pyscythe()
        .args(["deps", "--format", "json"])
        .arg(fixture("deps"))
        .output()
        .expect("runs");

    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(report["kind"], "deps");
    let mut findings: Vec<(String, String)> = report["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|f| {
            (
                f["rule"].as_str().expect("rule").to_owned(),
                f["message"].as_str().expect("message").to_owned(),
            )
        })
        .collect();
    findings.sort();
    assert_eq!(
        findings,
        [
            (
                "unresolved-import".to_owned(),
                "import `nowhere_to_be_found` resolves to nothing on the search path".to_owned()
            ),
            (
                "unused-dependency".to_owned(),
                "dependency `requests` is never imported".to_owned()
            ),
        ],
        "typing-extensions is matched by name, ruff is a tool, json is stdlib: {report}"
    );
}
