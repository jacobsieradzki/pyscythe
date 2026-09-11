//! Acceptance tests for CI-facing features: suppression comments, baselines,
//! confidence thresholds, and machine-readable output formats.

use predicates::prelude::*;

use support::{fixture, json_report, pyscythe, symbols};

/// Helpers live in a test module so the test-only lint allowances apply.
#[cfg(test)]
mod support {
    use std::path::PathBuf;

    use assert_cmd::Command;

    pub(crate) fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    pub(crate) fn pyscythe() -> Command {
        Command::new(env!("CARGO_BIN_EXE_pyscythe"))
    }

    pub(crate) fn json_report(args: &[&str], fixture_name: &str) -> serde_json::Value {
        let output = pyscythe()
            .args(args)
            .arg(fixture(fixture_name))
            .output()
            .expect("runs");
        serde_json::from_slice(&output.stdout).expect("valid json")
    }

    pub(crate) fn symbols(report: &serde_json::Value) -> Vec<&str> {
        report["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .filter_map(|f| f["symbol"].as_str())
            .collect()
    }
}

#[test]
fn suppression_comments_silence_findings_and_whole_files() {
    let report = json_report(&["dead-code", "--format", "json"], "suppressed");
    assert_eq!(symbols(&report), ["wrong_rule", "loud"], "{report}");
    assert_eq!(
        report["summary"]["suppressed"], 4,
        "two functions, plus legacy.py's symbol and file"
    );
    let file_reported = report["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .any(|f| f["rule"] == "unused-file");
    assert!(!file_reported, "legacy.py is ignored as a file: {report}");
}

#[test]
fn a_baseline_records_current_findings_and_hides_them_next_time() {
    let baseline =
        std::env::temp_dir().join(format!("pyscythe-baseline-{}.json", std::process::id()));
    let baseline_arg = baseline.to_str().expect("utf-8 temp path");

    pyscythe()
        .args(["dead-code", "--write-baseline", baseline_arg])
        .arg(fixture("simple_unused"))
        .assert()
        .success();

    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&baseline).expect("baseline written"))
            .expect("json");
    let recorded: Vec<&str> = written["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|k| k["path"].as_str().expect("path"))
        .collect();
    assert_eq!(
        recorded,
        ["pkg/helpers.py", "pkg/helpers.py"],
        "paths are project-relative"
    );

    let output = pyscythe()
        .args(["dead-code", "--format", "json", "--baseline", baseline_arg])
        .arg(fixture("simple_unused"))
        .output()
        .expect("runs");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(
        output.status.code(),
        Some(0),
        "everything is baselined: {report}"
    );
    assert!(symbols(&report).is_empty());
    assert_eq!(report["summary"]["baselined"], 2);

    std::fs::remove_file(baseline).ok();
}

#[test]
fn min_confidence_drops_lower_confidence_findings() {
    let report = json_report(
        &[
            "dead-code",
            "--format",
            "json",
            "--min-confidence",
            "medium",
        ],
        "methods",
    );
    let confidences: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter_map(|f| f["confidence"].as_str())
        .collect();
    assert!(!confidences.is_empty());
    assert!(confidences.iter().all(|c| *c != "low"), "{report}");
}

#[test]
fn sarif_output_has_rules_and_relative_locations() {
    let sarif = json_report(&["dead-code", "--format", "sarif"], "simple_unused");
    assert_eq!(sarif["version"], "2.1.0");
    let run = &sarif["runs"][0];
    assert_eq!(run["tool"]["driver"]["name"], "pyscythe");
    assert!(
        run["tool"]["driver"]["rules"]
            .as_array()
            .expect("rules")
            .len()
            >= 6
    );
    let results = run["results"].as_array().expect("results");
    assert_eq!(results.len(), 2, "{sarif}");
    let location = &results[0]["locations"][0]["physicalLocation"];
    assert_eq!(location["artifactLocation"]["uri"], "pkg/helpers.py");
    assert_eq!(location["region"]["startLine"], 5);
    assert_eq!(results[0]["ruleId"], "unused-function");
}

#[test]
fn github_output_emits_workflow_commands() {
    pyscythe()
        .args(["dead-code", "--format", "github"])
        .arg(fixture("simple_unused"))
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "::warning file=pkg/helpers.py,line=5,col=5,title=pyscythe unused-function::function `unused_helper` is never used",
        ));
}

#[test]
fn markdown_output_is_a_table() {
    pyscythe()
        .args(["dead-code", "--format", "markdown"])
        .arg(fixture("simple_unused"))
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "| Rule | Location | Finding | Confidence |",
        ))
        .stdout(predicate::str::contains(
            "| `unused-class` | `pkg/helpers.py:13` | class `UnusedThing` is never used | medium |",
        ));
}

#[test]
fn a_suppression_that_silences_nothing_is_reported() {
    let report = json_report(&["dead-code", "--format", "json"], "suppressed");
    let stale: Vec<u64> = report["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter(|f| f["rule"] == "unused-suppression")
        .filter_map(|f| f["position"]["line"].as_u64())
        .collect();
    assert_eq!(
        stale,
        [10, 19],
        "wrong-rule comment and the comment on a used function: {report}"
    );
}

#[test]
fn since_scopes_findings_to_files_changed_after_a_ref() {
    let scratch = std::env::temp_dir().join(format!("pyscythe-since-{}", std::process::id()));
    let pkg = scratch.join("pkg");
    std::fs::create_dir_all(&pkg).expect("scratch dir");
    std::fs::write(
        scratch.join("pyproject.toml"),
        "[project]\nname = \"since\"\nversion = \"0\"\nrequires-python = \">=3.12\"\n",
    )
    .expect("write");
    std::fs::write(pkg.join("__init__.py"), "").expect("write");
    std::fs::write(pkg.join("a.py"), "def old():\n    pass\n").expect("write");
    std::fs::write(pkg.join("b.py"), "def older():\n    pass\n").expect("write");
    std::fs::write(pkg.join("__main__.py"), "import pkg.a\nimport pkg.b\n").expect("write");

    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&scratch)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "base"]);
    std::fs::write(
        pkg.join("b.py"),
        "def older():\n    pass\n\n\ndef newer():\n    pass\n",
    )
    .expect("write");
    std::fs::write(pkg.join("c.py"), "def untracked():\n    pass\n").expect("write");

    let output = pyscythe()
        .args(["dead-code", "--format", "json", "--since", "HEAD"])
        .arg(&scratch)
        .output()
        .expect("runs");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");

    let mut files: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter_map(|f| f["module"].as_str())
        .collect();
    files.sort_unstable();
    files.dedup();
    assert_eq!(files, ["pkg.b", "pkg.c"], "a.py is unchanged: {report}");
    assert_eq!(report["summary"]["changed_files"], 2);

    std::fs::remove_dir_all(scratch).ok();
}

#[test]
fn since_with_a_bad_ref_is_an_error() {
    pyscythe()
        .args(["dead-code", "--since", "no-such-ref"])
        .arg(fixture("simple_unused"))
        .assert()
        .code(2)
        .stderr(predicate::str::contains("git diff failed"));
}
