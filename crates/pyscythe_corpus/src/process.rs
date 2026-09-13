//! Running external commands and turning failures into errors that name the command.

use std::process::Command;

use anyhow::{Context as _, Result, bail};

/// Runs the command to completion, failing with its stderr if it exits non-zero.
pub(crate) fn run(command: &mut Command) -> Result<()> {
    let description = describe(command);
    let output = command
        .output()
        .with_context(|| format!("starting `{description}`"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "`{description}` failed ({}):\n{}",
            output.status,
            stderr.trim_end()
        );
    }
    Ok(())
}

/// Runs the command and returns its trimmed stdout, failing if it exits non-zero.
pub(crate) fn output_of(command: &mut Command) -> Result<String> {
    let description = describe(command);
    let output = command
        .output()
        .with_context(|| format!("starting `{description}`"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "`{description}` failed ({}):\n{}",
            output.status,
            stderr.trim_end()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn describe(command: &Command) -> String {
    let arguments = command
        .get_args()
        .map(|argument| argument.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    format!("{} {arguments}", command.get_program().to_string_lossy())
}
