//! Building each project's virtual environment from its lock file with uv.
//!
//! The lock file pins every third-party distribution and every environment is
//! installed as if for Linux, so ty resolves the same packages on every
//! machine. The project's own code is installed editable without dependencies
//! afterwards, because a lock cannot pin a checkout.

use std::fs;
use std::io::Write;
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};

use crate::manifest::{Project, ProjectName, Requirement};
use crate::process;

const STAMP: &str = "pyscythe-corpus.stamp";

/// Every environment installs the packages a Linux machine would, whatever the host, so
/// platform-conditional dependencies (`sys_platform == "darwin"`) resolve the same everywhere.
/// ty only reads the files, so wheels built for another platform are no problem.
const TARGET_PLATFORM: &str = "linux";

/// The lock file for a project.
pub(crate) fn lock_file(locks: &Utf8Path, name: &ProjectName) -> Utf8PathBuf {
    locks.join(format!("{name}.txt"))
}

/// Resolves the project's requirements with uv and writes them, pinned, to the lock file.
pub(crate) fn lock(
    clone: &Utf8Path,
    project: &Project,
    lock_file: &Utf8Path,
    scratch: &Utf8Path,
) -> Result<()> {
    let root = clone.join(&project.root);
    let mut input = String::new();
    for requirement in &project.requirements {
        match requirement {
            Requirement::Specifier(specifier) => input.push_str(specifier),
            Requirement::File(file) => {
                input.push_str("-r ");
                input.push_str(root.join(file).as_str());
            }
        }
        input.push('\n');
    }
    for editable in &project.editable {
        input.push_str("-e ");
        input.push_str(clone.join(editable).as_str());
        input.push('\n');
    }
    let input_file = scratch.join("requirements.in");
    fs::write(&input_file, input).with_context(|| format!("writing {input_file}"))?;
    let resolved = process::output_of(
        uv(&root)
            .args(["pip", "compile", "--quiet", "--universal", "--no-sources"])
            .args(["--no-header", "--no-annotate"])
            .args(["--python", project.python.as_str()])
            .args(["--python-version", project.python.as_str()])
            .arg(&input_file),
    )?;
    let mut pinned = String::new();
    for line in resolved.lines() {
        if line.starts_with("-e ") || line.contains("file://") {
            continue;
        }
        pinned.push_str(line);
        pinned.push('\n');
    }
    if let Some(parent) = lock_file.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {parent}"))?;
    }
    fs::write(lock_file, pinned).with_context(|| format!("writing {lock_file}"))?;
    Ok(())
}

/// Creates `<root>/.venv` from the lock file unless a matching one already exists.
pub(crate) fn ensure(
    name: &ProjectName,
    clone: &Utf8Path,
    project: &Project,
    lock_file: &Utf8Path,
    out: &mut dyn Write,
) -> Result<()> {
    let lock = fs::read_to_string(lock_file).with_context(|| {
        format!("reading {lock_file}: run `pyscythe-corpus lock --project {name}` first")
    })?;
    let root = clone.join(&project.root);
    let venv = root.join(".venv");
    let stamp = stamp_for(project, &lock);
    if fs::read_to_string(venv.join(STAMP)).ok().as_deref() == Some(stamp.as_str()) {
        return Ok(());
    }
    writeln!(
        out,
        "{name}: building environment with Python {}",
        project.python.as_str()
    )?;
    if venv.exists() {
        fs::remove_dir_all(&venv).with_context(|| format!("removing {venv}"))?;
    }
    process::run(
        uv(&root)
            .args(["venv", "--quiet", "--python", project.python.as_str()])
            .arg(&venv),
    )?;
    process::run(
        uv(&root)
            .env("VIRTUAL_ENV", &venv)
            .args([
                "pip",
                "sync",
                "--quiet",
                "--python-platform",
                TARGET_PLATFORM,
            ])
            .arg(lock_file),
    )?;
    if !project.editable.is_empty() {
        let mut install = uv(&root);
        install.env("VIRTUAL_ENV", &venv).args([
            "pip",
            "install",
            "--quiet",
            "--no-deps",
            "--no-sources",
        ]);
        for editable in &project.editable {
            install.arg("-e").arg(clone.join(editable));
        }
        process::run(&mut install)?;
    }
    let python = venv.join("bin").join("python");
    if !python.is_file() {
        bail!("{name}: environment has no interpreter at {python}");
    }
    fs::write(venv.join(STAMP), stamp).with_context(|| format!("writing {STAMP} in {venv}"))?;
    Ok(())
}

fn stamp_for(project: &Project, lock: &str) -> String {
    let editable = project
        .editable
        .iter()
        .map(|path| path.as_str())
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "python={}\nplatform={TARGET_PLATFORM}\neditable={editable}\n{lock}",
        project.python.as_str()
    )
}

fn uv(cwd: &Utf8Path) -> Command {
    let mut command = Command::new("uv");
    command
        .current_dir(cwd)
        .arg("--no-config")
        // A shallow checkout has no tags for setuptools-scm and hatch-vcs to read.
        .env("SETUPTOOLS_SCM_PRETEND_VERSION", "0.0.0")
        // Only the Python files matter; skip optional C extensions.
        .env("DISABLE_SQLALCHEMY_CEXT", "1")
        .env("MYPY_USE_MYPYC", "0");
    command
}
