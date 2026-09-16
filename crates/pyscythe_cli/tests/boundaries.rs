//! Acceptance tests for `pyscythe boundaries`.

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
fn reports_upward_imports_and_denied_imports_but_not_type_only_ones() {
    let output = pyscythe()
        .args(["boundaries", "--format", "json"])
        .arg(fixture("boundaries"))
        .output()
        .expect("runs");

    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(report["kind"], "boundaries");
    let mut edges: Vec<(String, String)> = report["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|f| {
            (
                f["from_module"].as_str().expect("from").to_owned(),
                f["to_module"].as_str().expect("to").to_owned(),
            )
        })
        .collect();
    edges.sort();
    assert_eq!(
        edges,
        [
            ("app.domain.order".to_owned(), "app.util".to_owned()),
            (
                "app.services.orders".to_owned(),
                "app.api.routes".to_owned()
            ),
        ],
        "{report}"
    );
}

#[test]
fn human_output_explains_the_rule() {
    pyscythe()
        .arg("boundaries")
        .arg(fixture("boundaries"))
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "orders.py:1:1  `app.services.orders` imports `app.api.routes`: layer `app.services` may not depend on the layer above it, `app.api`",
        ))
        .stdout(predicate::str::contains("`app.domain` may not import `app.util`"));
}

#[test]
fn a_project_without_boundary_config_is_an_error() {
    pyscythe()
        .arg("boundaries")
        .arg(fixture("simple_unused"))
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no boundaries configured"));
}

#[test]
fn suggest_prints_a_layers_table_from_the_import_graph() {
    pyscythe()
        .args(["boundaries", "--suggest"])
        .arg(fixture("boundaries"))
        .assert()
        .success()
        .stdout(predicate::str::starts_with(
            "[tool.pyscythe.boundaries]\nlayers = [\n",
        ))
        .stdout(predicate::str::contains("[\"app.api\", \"app.services\"],"))
        .stdout(predicate::str::contains("import each other"));
}

#[test]
fn a_config_file_supplies_boundaries_a_project_does_not_declare() {
    let project = fixture("unconfigured_boundaries");
    pyscythe()
        .arg("boundaries")
        .arg(&project)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no boundaries configured"));

    let output = pyscythe()
        .args(["boundaries", "--format", "json", "--config"])
        .arg(fixture("configs/layered.toml"))
        .arg(&project)
        .output()
        .expect("runs");

    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let edges: Vec<(&str, &str)> = report["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|f| {
            (
                f["from_module"].as_str().expect("from"),
                f["to_module"].as_str().expect("to"),
            )
        })
        .collect();
    assert_eq!(
        edges,
        [("app.services.orders", "app.api.routes")],
        "{report}"
    );
}
