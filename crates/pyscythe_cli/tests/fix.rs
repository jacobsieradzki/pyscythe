//! Acceptance tests for `pyscythe fix`, run on scratch copies of fixtures.

use predicates::prelude::*;

use support::{copy_fixture, pyscythe};

#[cfg(test)]
mod support {
    use std::path::{Path, PathBuf};

    use assert_cmd::Command;

    pub(crate) fn pyscythe() -> Command {
        Command::new(env!("CARGO_BIN_EXE_pyscythe"))
    }

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).expect("create dir");
        for entry in std::fs::read_dir(from).expect("read dir") {
            let entry = entry.expect("entry");
            let target = to.join(entry.file_name());
            if entry.file_type().expect("type").is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).expect("copy");
            }
        }
    }

    static COPIES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    /// A private copy of a fixture project, so writes do not touch the tree
    /// and tests running in parallel do not share a directory.
    pub(crate) fn copy_fixture(name: &str) -> PathBuf {
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let copy = COPIES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let target =
            std::env::temp_dir().join(format!("pyscythe-fix-{name}-{}-{copy}", std::process::id()));
        std::fs::remove_dir_all(&target).ok();
        copy_dir(&source, &target);
        target
    }
}

#[test]
fn dry_run_shows_a_diff_and_changes_nothing() {
    let project = copy_fixture("simple_unused");
    let helpers = project.join("pkg/helpers.py");
    let before = std::fs::read_to_string(&helpers).expect("read");

    pyscythe()
        .args(["fix", "--dry-run"])
        .arg(&project)
        .assert()
        .success()
        .stdout(predicate::str::contains("--- a/pkg/helpers.py"))
        .stdout(predicate::str::contains("-def unused_helper() -> int:"))
        .stdout(predicate::str::contains("-class UnusedThing:"))
        .stdout(predicate::str::contains(
            "Would remove 2 definition(s) in 1 file(s) and 0 whole file(s)",
        ));

    assert_eq!(std::fs::read_to_string(&helpers).expect("read"), before);
    std::fs::remove_dir_all(project).ok();
}

#[test]
fn applying_removes_definitions_and_leaves_the_project_clean() {
    let project = copy_fixture("simple_unused");

    pyscythe()
        .arg("fix")
        .arg(&project)
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed 2 definition(s)"));

    let helpers = std::fs::read_to_string(project.join("pkg/helpers.py")).expect("read");
    assert_eq!(
        helpers,
        "def used_helper() -> int:\n    return 1\n\n\nclass UsedThing:\n    pass\n"
    );
    pyscythe().arg("dead-code").arg(&project).assert().success();
    std::fs::remove_dir_all(project).ok();
}

#[test]
fn unused_files_are_deleted_and_scripts_kept() {
    let project = copy_fixture("unused_file");

    pyscythe()
        .arg("fix")
        .arg(&project)
        .assert()
        .success()
        .stdout(predicate::str::contains("delete pkg/orphan.py"));

    assert!(!project.join("pkg/orphan.py").exists());
    assert!(project.join("scripts/backfill.py").exists());
    std::fs::remove_dir_all(project).ok();
}

#[test]
fn low_confidence_findings_are_left_alone_by_default() {
    let project = copy_fixture("methods");

    pyscythe()
        .args(["fix", "--dry-run"])
        .arg(&project)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "-    def _unused_private(self) -> None:",
        ))
        .stdout(predicate::str::contains("-    def unused_public(self) -> None:").not());
    std::fs::remove_dir_all(project).ok();
}
