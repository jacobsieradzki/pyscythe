//! The pyscythe subcommands the corpus runs.

use std::fmt;

use clap::ValueEnum;

/// A read-only pyscythe analysis with a JSON report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub(crate) enum Analysis {
    DeadCode,
    Cycles,
    Deps,
    Dupes,
    Health,
    /// Only for projects whose `boundaries` config says what their layers are.
    Boundaries,
    /// What `fix` would remove, as a dry run: the plan, not the diff.
    Fix,
}

impl Analysis {
    pub(crate) const ALL: [Self; 7] = [
        Self::DeadCode,
        Self::Cycles,
        Self::Deps,
        Self::Dupes,
        Self::Health,
        Self::Boundaries,
        Self::Fix,
    ];

    /// The subcommand name as typed on the command line.
    pub(crate) const fn subcommand(self) -> &'static str {
        match self {
            Self::DeadCode => "dead-code",
            Self::Cycles => "cycles",
            Self::Deps => "deps",
            Self::Dupes => "dupes",
            Self::Health => "health",
            Self::Boundaries => "boundaries",
            Self::Fix => "fix",
        }
    }

    /// Arguments the subcommand needs beyond a path and a format.
    pub(crate) const fn extra_arguments(self) -> &'static [&'static str] {
        match self {
            Self::Fix => &["--dry-run"],
            _ => &[],
        }
    }

    /// The snapshot file name inside a project's snapshot directory.
    pub(crate) fn snapshot_file(self) -> String {
        format!("{}.txt", self.subcommand())
    }
}

impl fmt::Display for Analysis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.subcommand())
    }
}
