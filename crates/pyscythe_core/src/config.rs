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
    patterns: Vec<String>,
}

impl PathPatterns {
    /// Matches nothing.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            set: GlobSet::empty(),
            patterns: Vec::new(),
        }
    }

    /// These patterns plus `more`.
    ///
    /// # Errors
    ///
    /// Returns the first pattern in `more` that fails to parse.
    pub fn extended<'a>(
        &'a self,
        more: impl IntoIterator<Item = &'a str>,
    ) -> Result<Self, PatternError> {
        Self::parse(self.patterns.iter().map(String::as_str).chain(more))
    }

    /// Compiles `patterns`; `**` crosses directories and a bare `dir/` prefix
    /// such as `scripts` matches everything underneath it.
    ///
    /// # Errors
    ///
    /// Returns the first pattern that fails to parse.
    pub fn parse<'a>(patterns: impl IntoIterator<Item = &'a str>) -> Result<Self, PatternError> {
        let mut builder = GlobSetBuilder::new();
        let mut kept = Vec::new();
        for pattern in patterns {
            kept.push(pattern.to_owned());
            builder.add(glob(pattern)?);
            if !pattern.contains('*') {
                builder.add(glob(&format!("{}/**", pattern.trim_end_matches('/')))?);
            }
        }
        let set = builder.build().map_err(|source| PatternError {
            pattern: String::from("<set>"),
            source,
        })?;
        Ok(Self {
            set,
            patterns: kept,
        })
    }

    /// Whether `relative_path` matches any pattern.
    #[must_use]
    pub fn matches(&self, relative_path: &Utf8Path) -> bool {
        !self.patterns.is_empty() && self.set.is_match(relative_path.as_str())
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

/// A module and everything beneath it: `app.domain` covers `app.domain.orders`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModulePrefix(String);

impl ModulePrefix {
    /// Wraps a dotted module path.
    #[must_use]
    pub fn new(dotted: impl Into<String>) -> Self {
        Self(dotted.into())
    }

    /// The prefix as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `module` is this module or lives inside it.
    #[must_use]
    pub fn covers(&self, module: &str) -> bool {
        module == self.0
            || module
                .strip_prefix(self.0.as_str())
                .is_some_and(|rest| rest.starts_with('.'))
    }
}

/// Whether imports under `if TYPE_CHECKING:` are held to boundary rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeOnlyImports {
    /// Type-only imports may cross boundaries; they never run.
    Allow,
    /// Type-only imports are checked like any other.
    Check,
}

/// One explicit prohibition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenyRule {
    /// Modules the rule applies to.
    pub from: ModulePrefix,
    /// Modules they may not import.
    pub deny: Vec<ModulePrefix>,
    /// Exceptions inside `deny`, such as a types-only submodule.
    pub allow: Vec<ModulePrefix>,
}

/// Architecture boundaries from `[tool.pyscythe.boundaries]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundaryConfig {
    /// Ranks from top to bottom; a module may import its own rank and any rank below.
    /// Several prefixes may share a rank.
    pub layers: Vec<Vec<ModulePrefix>>,
    /// Explicit prohibitions, checked in addition to the layers.
    pub rules: Vec<DenyRule>,
    /// How type-only imports are treated.
    pub type_only: TypeOnlyImports,
}

impl BoundaryConfig {
    /// Whether anything is configured at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.layers.is_empty() && self.rules.is_empty()
    }

    /// The hexagonal preset under `root`: adapters, api, and infrastructure on
    /// top; application beneath; domain at the bottom importing nothing above it.
    #[must_use]
    pub fn hexagonal(root: &str) -> Self {
        let under = |name: &str| ModulePrefix::new(format!("{root}.{name}"));
        Self {
            layers: vec![
                vec![
                    under("adapters"),
                    under("api"),
                    under("infrastructure"),
                    under("infra"),
                ],
                vec![under("application"), under("services")],
                vec![under("domain")],
            ],
            rules: Vec::new(),
            type_only: TypeOnlyImports::Allow,
        }
    }

    /// The rank of `module`, if it lives in a configured layer.
    #[must_use]
    pub fn rank_of(&self, module: &str) -> Option<usize> {
        self.layers
            .iter()
            .position(|rank| rank.iter().any(|prefix| prefix.covers(module)))
    }
}

/// Where a function starts costing health points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthThresholds {
    /// Above this cyclomatic complexity a function is a hotspot.
    pub max_cyclomatic: u32,
    /// Above this cognitive complexity a function is a hotspot.
    pub max_cognitive: u32,
    /// Above this many lines a function loses points.
    pub max_lines: u32,
    /// Above this many parameters a function loses points.
    pub max_parameters: u32,
}

impl Default for HealthThresholds {
    fn default() -> Self {
        Self {
            max_cyclomatic: 10,
            max_cognitive: 15,
            max_lines: 50,
            max_parameters: 6,
        }
    }
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
    /// Architecture boundaries, when configured.
    pub boundaries: Option<BoundaryConfig>,
    /// Health thresholds.
    pub health: HealthThresholds,
    /// Distributions never reported as unused, on top of the built-in tool list.
    pub ignored_dependencies: Vec<crate::manifest::DistributionName>,
    /// Modules whose public names are the project's API and so never dead:
    /// what a library exports to the world.
    pub public_modules: Vec<ModulePrefix>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            exclude: PathPatterns::none(),
            ignore_names: NamePatterns::none(),
            notebooks: NotebookPolicy::Exclude,
            boundaries: None,
            health: HealthThresholds::default(),
            ignored_dependencies: Vec::new(),
            public_modules: Vec::new(),
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
    fn patterns_can_be_extended() {
        let base = PathPatterns::parse(["scripts"]).unwrap();
        let extended = base.extended(["docs"]).unwrap();
        assert!(extended.matches(Utf8Path::new("scripts/a.py")));
        assert!(extended.matches(Utf8Path::new("docs/conf.py")));
        assert!(!base.matches(Utf8Path::new("docs/conf.py")));
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
    fn module_prefixes_cover_themselves_and_submodules_only() {
        let prefix = super::ModulePrefix::new("app.domain");
        assert!(prefix.covers("app.domain"));
        assert!(prefix.covers("app.domain.orders"));
        assert!(!prefix.covers("app.domainx"));
        assert!(!prefix.covers("app"));
    }

    #[test]
    fn hexagonal_preset_ranks_domain_lowest() {
        let config = super::BoundaryConfig::hexagonal("app");
        assert_eq!(config.rank_of("app.api.routes"), Some(0));
        assert_eq!(config.rank_of("app.services.orders"), Some(1));
        assert_eq!(config.rank_of("app.domain.order"), Some(2));
        assert_eq!(config.rank_of("app.cli"), None);
    }

    #[test]
    fn a_bad_pattern_is_an_error() {
        assert!(NamePatterns::parse(["[unclosed"]).is_err());
    }
}
