//! User configuration from `[tool.pyscythe]`.

use camino::Utf8Path;
use globset::{Glob, GlobSet, GlobSetBuilder};

/// A pattern that did not parse.
#[derive(Debug, thiserror::Error)]
#[error("invalid glob pattern `{pattern}`: {source}")]
pub struct PatternError {
    /// The offending pattern.
    pub pattern: String,
    /// Why it failed.
    #[source]
    pub source: globset::Error,
}

/// Globs matched against project-relative file paths.
#[derive(Debug, Clone)]
pub struct PathPatterns {
    set: GlobSet,
    is_empty: bool,
}

impl PathPatterns {
    /// Matches nothing.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            set: GlobSet::empty(),
            is_empty: true,
        }
    }

    /// Compiles `patterns`; `**` crosses directories and a bare `dir/` prefix
    /// such as `scripts` matches everything underneath it.
    ///
    /// # Errors
    ///
    /// Returns the first pattern that fails to parse.
    pub fn parse<'a>(patterns: impl IntoIterator<Item = &'a str>) -> Result<Self, PatternError> {
        let mut builder = GlobSetBuilder::new();
        let mut is_empty = true;
        for pattern in patterns {
            is_empty = false;
            builder.add(glob(pattern)?);
            if !pattern.contains('*') {
                builder.add(glob(&format!("{}/**", pattern.trim_end_matches('/')))?);
            }
        }
        let set = builder.build().map_err(|source| PatternError {
            pattern: String::from("<set>"),
            source,
        })?;
        Ok(Self { set, is_empty })
    }

    /// Whether `relative_path` matches any pattern.
    #[must_use]
    pub fn matches(&self, relative_path: &Utf8Path) -> bool {
        !self.is_empty && self.set.is_match(relative_path.as_str())
    }
}

/// Globs matched against symbol names.
#[derive(Debug, Clone)]
pub struct NamePatterns {
    set: GlobSet,
    is_empty: bool,
}

impl NamePatterns {
    /// Matches nothing.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            set: GlobSet::empty(),
            is_empty: true,
        }
    }

    /// Compiles `patterns`, such as `legacy_*` or `*_v1`.
    ///
    /// # Errors
    ///
    /// Returns the first pattern that fails to parse.
    pub fn parse<'a>(patterns: impl IntoIterator<Item = &'a str>) -> Result<Self, PatternError> {
        let mut builder = GlobSetBuilder::new();
        let mut is_empty = true;
        for pattern in patterns {
            is_empty = false;
            builder.add(glob(pattern)?);
        }
        let set = builder.build().map_err(|source| PatternError {
            pattern: String::from("<set>"),
            source,
        })?;
        Ok(Self { set, is_empty })
    }

    /// Whether `name` matches any pattern.
    #[must_use]
    pub fn matches(&self, name: &str) -> bool {
        !self.is_empty && self.set.is_match(name)
    }
}

fn glob(pattern: &str) -> Result<Glob, PatternError> {
    Glob::new(pattern).map_err(|source| PatternError {
        pattern: pattern.to_owned(),
        source,
    })
}

/// Whether notebooks take part in analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotebookPolicy {
    /// Skip `.ipynb` files.
    Exclude,
    /// Analyse notebook cells like modules.
    Include,
}

/// Everything the user can tune.
#[derive(Debug, Clone)]
pub struct Config {
    /// Files to leave out of reports. Their references still count.
    pub exclude: PathPatterns,
    /// Symbol names never to report.
    pub ignore_names: NamePatterns,
    /// Whether notebooks are analysed.
    pub notebooks: NotebookPolicy,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            exclude: PathPatterns::none(),
            ignore_names: NamePatterns::none(),
            notebooks: NotebookPolicy::Exclude,
        }
    }
}

#[cfg(test)]
mod tests {
    use camino::Utf8Path;

    use super::{NamePatterns, PathPatterns};

    #[test]
    fn a_bare_directory_pattern_matches_everything_beneath_it() {
        let patterns = PathPatterns::parse(["scripts"]).unwrap();
        assert!(patterns.matches(Utf8Path::new("scripts/one_off.py")));
        assert!(patterns.matches(Utf8Path::new("scripts/nested/deep.py")));
        assert!(!patterns.matches(Utf8Path::new("pkg/scripts.py")));
    }

    #[test]
    fn globs_cross_directories_with_double_star() {
        let patterns = PathPatterns::parse(["**/legacy_*.py"]).unwrap();
        assert!(patterns.matches(Utf8Path::new("pkg/sub/legacy_thing.py")));
        assert!(!patterns.matches(Utf8Path::new("pkg/sub/thing.py")));
    }

    #[test]
    fn empty_patterns_match_nothing() {
        assert!(!PathPatterns::none().matches(Utf8Path::new("anything.py")));
        assert!(!NamePatterns::none().matches("anything"));
    }

    #[test]
    fn name_patterns_match_whole_names() {
        let patterns = NamePatterns::parse(["legacy_*", "*_v1"]).unwrap();
        assert!(patterns.matches("legacy_handler"));
        assert!(patterns.matches("parse_v1"));
        assert!(!patterns.matches("handler"));
    }

    #[test]
    fn a_bad_pattern_is_an_error() {
        assert!(NamePatterns::parse(["[unclosed"]).is_err());
    }
}
