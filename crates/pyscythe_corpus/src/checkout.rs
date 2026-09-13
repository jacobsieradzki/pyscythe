//! Fetching each project at its pinned commit into the cache.

use std::fs;
use std::io::Write;
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};

use crate::manifest::{Project, ProjectName};
use crate::process;

/// Makes `<cache>/<name>` a checkout of the project's pinned commit and returns that directory.
pub(crate) fn ensure(
    cache: &Utf8Path,
    name: &ProjectName,
    project: &Project,
    out: &mut dyn Write,
) -> Result<Utf8PathBuf> {
    let clone = cache.join(name.as_str());
    if !clone.join(".git").is_dir() {
        fs::create_dir_all(&clone).with_context(|| format!("creating {clone}"))?;
        process::run(git(&clone).args(["init", "-q"]))?;
        process::run(git(&clone).args(["remote", "add", "origin", project.repository.as_str()]))?;
    }
    if head_of(&clone).as_deref() == Some(project.commit.as_str()) {
        return Ok(clone);
    }
    writeln!(out, "{name}: fetching {}", project.commit.as_str())?;
    process::run(git(&clone).args([
        "fetch",
        "-q",
        "--depth",
        "1",
        "origin",
        project.commit.as_str(),
    ]))?;
    process::run(git(&clone).args([
        "-c",
        "advice.detachedHead=false",
        "checkout",
        "-q",
        "--force",
        "FETCH_HEAD",
    ]))?;
    let head = head_of(&clone);
    if head.as_deref() != Some(project.commit.as_str()) {
        bail!(
            "{name}: checked out {} but wanted {}",
            head.unwrap_or_default(),
            project.commit.as_str()
        );
    }
    Ok(clone)
}

fn head_of(clone: &Utf8Path) -> Option<String> {
    process::output_of(git(clone).args(["rev-parse", "--verify", "-q", "HEAD"])).ok()
}

fn git(clone: &Utf8Path) -> Command {
    let mut command = Command::new("git");
    command.current_dir(clone);
    command
}
