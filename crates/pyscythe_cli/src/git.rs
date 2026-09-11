//! Which files changed since a git ref, for scoping a run to a pull request.

use std::collections::BTreeSet;
use std::process::Command;

use camino::{Utf8Path, Utf8PathBuf};

/// Why the changed-file set could not be computed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum GitError {
    /// `git` could not be started.
    #[error("cannot run git: {0}")]
    Spawn(#[source] std::io::Error),
    /// `git` ran and refused: not a repository, unknown ref, and so on.
    #[error("git {command} failed: {stderr}")]
    Failed {
        /// The subcommand that failed.
        command: &'static str,
        /// What git said.
        stderr: String,
    },
}

/// Absolute paths of files that differ from `reference`, plus untracked files.
pub(crate) fn changed_files(
    root: &Utf8Path,
    reference: &str,
) -> Result<BTreeSet<Utf8PathBuf>, GitError> {
    let repo_root = run(root, "rev-parse", &["--show-toplevel"])?;
    let repo_root = Utf8PathBuf::from(repo_root.trim());

    let mut files = BTreeSet::new();
    for listing in [
        run(root, "diff", &["--name-only", reference, "--"])?,
        run(
            root,
            "ls-files",
            &["--others", "--exclude-standard", "--full-name"],
        )?,
    ] {
        files.extend(
            listing
                .lines()
                .filter(|line| !line.is_empty())
                .map(|line| canonical(&repo_root.join(line))),
        );
    }
    Ok(files)
}

fn run(root: &Utf8Path, command: &'static str, args: &[&str]) -> Result<String, GitError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root.as_str())
        .arg(command)
        .args(args)
        .output()
        .map_err(GitError::Spawn)?;
    if !output.status.success() {
        return Err(GitError::Failed {
            command,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The canonical form of `path`, or `path` itself when it cannot be resolved.
pub(crate) fn canonical(path: &Utf8Path) -> Utf8PathBuf {
    path.canonicalize_utf8()
        .unwrap_or_else(|_| path.to_path_buf())
}
