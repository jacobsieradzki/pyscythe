//! Applies `pyscythe fix` to a project and checks that nothing it removed was
//! alive.
//!
//! A snapshot says whether the plan changed; it cannot say whether the plan is
//! safe. The only honest answer to that is to carry the plan out and ask the
//! interpreter: a package that imported before must still import after.
//!
//! The environments are pinned to Linux wheels so that every machine analyses
//! the same files, which means a package with a compiled dependency cannot be
//! imported anywhere else. On another platform those projects are reported as
//! proving nothing rather than as passing.

use std::fs;
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use camino::Utf8Path;

/// What happened when a project's dead code was actually removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Repair {
    /// Packages that imported before the fix.
    pub(crate) imported_before: Vec<String>,
    /// Of those, the ones that no longer import.
    pub(crate) broken: Vec<String>,
    /// Packages that did not import even before, so they prove nothing.
    pub(crate) unimportable: Vec<String>,
    /// What pyscythe said it removed.
    pub(crate) summary: String,
}

impl Repair {
    pub(crate) const fn is_success(&self) -> bool {
        self.broken.is_empty()
    }
}

/// Runs `fix` in `root`, imports every package it ships, and restores `clone`.
///
/// The checkout is restored with `git checkout`, never `git clean`: the
/// environment beside it is untracked and costs minutes to rebuild.
pub(crate) fn repair(binary: &Utf8Path, clone: &Utf8Path, root: &Utf8Path) -> Result<Repair> {
    let packages = packages_in(root);
    let imported_before = importable(root, &packages);
    let unimportable = packages
        .iter()
        .filter(|package| !imported_before.contains(package))
        .cloned()
        .collect();

    // Whatever happens, the checkout goes back to the pinned commit: the next
    // analysis must see the project as published.
    let outcome =
        run_fix(binary, root).map(|summary| (summary, importable(root, &imported_before)));
    restore(clone)?;
    let (summary, imported_after) = outcome?;

    Ok(Repair {
        broken: imported_before
            .iter()
            .filter(|package| !imported_after.contains(package))
            .cloned()
            .collect(),
        imported_before,
        unimportable,
        summary,
    })
}

/// Every top-level package under `root`: flat, `src/`, and the `lib/` that
/// ansible and a few others put their package in.
fn packages_in(root: &Utf8Path) -> Vec<String> {
    let mut out = Vec::new();
    for directory in [root.to_path_buf(), root.join("src"), root.join("lib")] {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if name.starts_with('.') || !entry.path().join("__init__.py").is_file() {
                continue;
            }
            out.push(name);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// The packages that import cleanly in the project's own environment.
fn importable(root: &Utf8Path, packages: &[String]) -> Vec<String> {
    let python = root.join(".venv").join("bin").join("python");
    packages
        .iter()
        .filter(|package| {
            Command::new(&python)
                .args(["-c", &format!("import {package}")])
                .current_dir(root)
                .env_remove("VIRTUAL_ENV")
                .env_remove("CONDA_PREFIX")
                .env_remove("PYTHONPATH")
                .output()
                .is_ok_and(|output| output.status.success())
        })
        .cloned()
        .collect()
}

fn run_fix(binary: &Utf8Path, root: &Utf8Path) -> Result<String> {
    let output = Command::new(binary)
        .arg("fix")
        .arg(root)
        .current_dir(root)
        .env_remove("VIRTUAL_ENV")
        .env_remove("CONDA_PREFIX")
        .output()
        .with_context(|| format!("running {binary} fix"))?;
    if !matches!(output.status.code(), Some(0 | 1)) {
        bail!(
            "pyscythe fix exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text
        .lines()
        .last()
        .unwrap_or("removed nothing")
        .trim()
        .to_owned())
}

fn restore(clone: &Utf8Path) -> Result<()> {
    let output = Command::new("git")
        .args(["checkout", "--", "."])
        .current_dir(clone)
        .output()
        .context("restoring the checkout")?;
    if !output.status.success() {
        bail!(
            "could not restore {clone}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}
