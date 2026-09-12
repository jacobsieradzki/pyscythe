//! pytest's collection settings, read from wherever pytest itself would:
//! `pytest.ini` first, then `[tool.pytest.ini_options]` in `pyproject.toml`,
//! then `tox.ini`, then `setup.cfg`; the first that configures pytest wins.

use std::collections::BTreeMap;

use camino::Utf8Path;
use pyscythe_core::config::{PatternError, TestCollection};
use serde::Deserialize;

/// Whether the root `pyproject.toml` carried a `[tool.pytest.ini_options]` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyprojectPytest {
    /// It did, so the ini files below it in precedence do not apply.
    Configured,
    /// It did not.
    Absent,
}

/// `[tool.pytest.ini_options]`, the keys that shape collection.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct IniOptions {
    #[serde(default)]
    python_files: Option<OneOrMany>,
    #[serde(default)]
    python_classes: Option<OneOrMany>,
    #[serde(default)]
    python_functions: Option<OneOrMany>,
}

/// pytest accepts `"a b"` or `["a", "b"]` for these keys.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn entries(&self) -> Vec<String> {
        match self {
            Self::One(text) => split_entries(text),
            Self::Many(list) => list.clone(),
        }
    }
}

/// Splits an ini value on whitespace and commas.
fn split_entries(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || c == ',')
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A collection from the three optional entry lists, defaults filling gaps.
fn collection(
    files: Option<Vec<String>>,
    classes: Option<Vec<String>>,
    functions: Option<Vec<String>>,
) -> Result<TestCollection, PatternError> {
    let or_default = |entries: Option<Vec<String>>, defaults: &[&str]| {
        entries.unwrap_or_else(|| defaults.iter().map(|s| (*s).to_owned()).collect())
    };
    let files = or_default(files, TestCollection::DEFAULT_FILES);
    let classes = or_default(classes, TestCollection::DEFAULT_CLASSES);
    let functions = or_default(functions, TestCollection::DEFAULT_FUNCTIONS);
    TestCollection::parse(
        files.iter().map(String::as_str),
        classes.iter().map(String::as_str),
        functions.iter().map(String::as_str),
    )
}

/// The collection a `[tool.pytest.ini_options]` table configures.
pub(crate) fn from_ini_options(options: &IniOptions) -> Result<TestCollection, PatternError> {
    collection(
        options.python_files.as_ref().map(OneOrMany::entries),
        options.python_classes.as_ref().map(OneOrMany::entries),
        options.python_functions.as_ref().map(OneOrMany::entries),
    )
}

/// The collection in force at `root`, given what its `pyproject.toml` said.
pub(crate) fn resolve(
    root: &Utf8Path,
    pyproject: PyprojectPytest,
    from_pyproject: TestCollection,
) -> Result<TestCollection, PatternError> {
    if let Some(section) = ini_section(&root.join("pytest.ini"), "pytest") {
        return from_ini_section(&section);
    }
    if pyproject == PyprojectPytest::Configured {
        return Ok(from_pyproject);
    }
    if let Some(section) = ini_section(&root.join("tox.ini"), "pytest") {
        return from_ini_section(&section);
    }
    if let Some(section) = ini_section(&root.join("setup.cfg"), "tool:pytest") {
        return from_ini_section(&section);
    }
    Ok(from_pyproject)
}

fn from_ini_section(section: &BTreeMap<String, String>) -> Result<TestCollection, PatternError> {
    let entries = |key: &str| section.get(key).map(|value| split_entries(value));
    collection(
        entries("python_files"),
        entries("python_classes"),
        entries("python_functions"),
    )
}

/// The `key = value` pairs of `[name]` in the ini file at `path`; `None` when
/// the file or the section is missing.
fn ini_section(path: &Utf8Path, name: &str) -> Option<BTreeMap<String, String>> {
    let text = std::fs::read_to_string(path).ok()?;
    ini_section_in(&text, name)
}

/// [`ini_section`] over text. Indented lines continue the previous value.
fn ini_section_in(text: &str, name: &str) -> Option<BTreeMap<String, String>> {
    let header = format!("[{name}]");
    let mut in_section = false;
    let mut found = false;
    let mut pairs: BTreeMap<String, String> = BTreeMap::new();
    let mut last_key: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = trimmed == header;
            found |= in_section;
            last_key = None;
            continue;
        }
        if !in_section || trimmed.is_empty() || trimmed.starts_with(['#', ';']) {
            continue;
        }
        if line.starts_with(char::is_whitespace)
            && let Some(key) = &last_key
            && let Some(value) = pairs.get_mut(key)
        {
            if !value.is_empty() {
                value.push(' ');
            }
            value.push_str(trimmed);
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            let key = key.trim().to_owned();
            pairs.insert(key.clone(), value.trim().to_owned());
            last_key = Some(key);
        }
    }
    found.then_some(pairs)
}

#[cfg(test)]
mod tests {
    use super::{from_ini_section, ini_section_in, split_entries};

    #[test]
    fn reads_a_section_with_continuation_lines() {
        let text = "[flake8]\nmax-line-length = 100\n\n[tool:pytest]\npython_files =\n    check_*.py\n    test_*.py\npython_functions = check_ test\n";
        let section = ini_section_in(text, "tool:pytest").unwrap();
        assert_eq!(section["python_files"], "check_*.py test_*.py");
        assert_eq!(section["python_functions"], "check_ test");
        assert!(ini_section_in(text, "pytest").is_none());
    }

    #[test]
    fn a_section_configures_collection_with_defaults_for_missing_keys() {
        let text = "[pytest]\npython_files = check_*.py\n";
        let section = ini_section_in(text, "pytest").unwrap();
        let tests = from_ini_section(&section).unwrap();
        assert!(tests.is_test_file("check_it.py"));
        assert!(!tests.is_test_file("test_it.py"));
        assert!(
            tests.collects_function("test_it"),
            "functions keep the default"
        );
    }

    #[test]
    fn entries_split_on_whitespace_and_commas() {
        assert_eq!(split_entries("a b,c\n d"), ["a", "b", "c", "d"]);
    }
}
