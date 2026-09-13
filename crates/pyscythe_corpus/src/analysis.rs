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
}

impl Analysis {
    pub(crate) const ALL: [Self; 5] = [
        Self::DeadCode,
        Self::Cycles,
        Self::Deps,
        Self::Dupes,
        Self::Health,
    ];

    /// The subcommand name as typed on the command line.
    pub(crate) const fn subcommand(self) -> &'static str {
        match self {
            Self::DeadCode => "dead-code",
            Self::Cycles => "cycles",
            Self::Deps => "deps",
            Self::Dupes => "dupes",
            Self::Health => "health",
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
