//! Files, modules, byte spans, and human-facing positions.

use std::num::NonZeroU32;

use camino::Utf8PathBuf;
use serde::Serialize;

/// Identifies one source file within a [`crate::index::CodebaseIndex`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(u32);

impl FileId {
    /// Wraps a raw index handed out by an index implementation.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw index, for adapters that keep files in a vector.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// A dotted Python module path such as `pkg.sub.module`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct ModulePath(String);

impl ModulePath {
    /// Wraps an already-dotted module path.
    #[must_use]
    pub fn new(dotted: impl Into<String>) -> Self {
        Self(dotted.into())
    }

    /// The dotted path as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Whether a module guards script behaviour behind `if __name__ == "__main__":`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MainGuard {
    /// The module can be run as a script.
    Present,
    /// No such guard.
    Absent,
}

/// A Python source file known to the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// Identity within the index.
    pub id: FileId,
    /// Absolute path on disk.
    pub path: Utf8PathBuf,
    /// The module this file resolves to, when it lives on a search path.
    pub module: Option<ModulePath>,
    /// Whether the file has a `__main__` guard.
    pub main_guard: MainGuard,
    /// Names listed in the file's `__all__`, in order.
    pub exports: Vec<crate::symbol::SymbolName>,
}

impl SourceFile {
    /// The file's name without directories, or empty when it has none.
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.path.file_name().unwrap_or_default()
    }
}

/// A byte offset into a file's source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ByteOffset(u32);

impl ByteOffset {
    /// Wraps a raw byte offset.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw byte offset.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// A half-open byte range `[start, end)` within one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ByteSpan {
    start: ByteOffset,
    end: ByteOffset,
}

impl ByteSpan {
    /// Builds a span, normalising the bounds so `start <= end` always holds.
    #[must_use]
    pub fn new(a: ByteOffset, b: ByteOffset) -> Self {
        Self {
            start: a.min(b),
            end: a.max(b),
        }
    }

    /// First byte of the span.
    #[must_use]
    pub const fn start(self) -> ByteOffset {
        self.start
    }

    /// One past the last byte of the span.
    #[must_use]
    pub const fn end(self) -> ByteOffset {
        self.end
    }

    /// Whether `other` lies entirely within this span.
    #[must_use]
    pub fn encloses(self, other: Self) -> bool {
        self.start <= other.start && other.end <= self.end
    }
}

/// A one-based line number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Line(NonZeroU32);

impl Line {
    /// Builds a line from a one-based number, rejecting zero.
    #[must_use]
    pub const fn from_one_based(n: u32) -> Option<Self> {
        match NonZeroU32::new(n) {
            Some(n) => Some(Self(n)),
            None => None,
        }
    }

    /// The one-based line number.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

/// A one-based column number, counted in characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Column(NonZeroU32);

impl Column {
    /// Builds a column from a one-based number, rejecting zero.
    #[must_use]
    pub const fn from_one_based(n: u32) -> Option<Self> {
        match NonZeroU32::new(n) {
            Some(n) => Some(Self(n)),
            None => None,
        }
    }

    /// The one-based column number.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

/// A human-facing line and column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct Position {
    /// One-based line.
    pub line: Line,
    /// One-based column.
    pub column: Column,
}
