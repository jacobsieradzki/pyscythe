//! Which installed distribution provides which top-level modules, read from
//! the `RECORD` and `top_level.txt` files in `*.dist-info` directories.

use std::collections::BTreeMap;

use camino::{Utf8Path, Utf8PathBuf};
use pyscythe_core::manifest::DistributionName;

/// Top-level module -> owning distributions, per site-packages directory.
#[derive(Debug, Default)]
pub(crate) struct DistributionIndex {
    by_site_packages: BTreeMap<Utf8PathBuf, BTreeMap<String, Vec<DistributionName>>>,
}

impl DistributionIndex {
    /// The distributions providing `top_level` under the site-packages
    /// directory that contains `module_file`; empty when no metadata is found.
    pub(crate) fn owners(
        &mut self,
        module_file: &Utf8Path,
        top_level: &str,
    ) -> Vec<DistributionName> {
        let Some(site_packages) = site_packages_of(module_file, top_level) else {
            return Vec::new();
        };
        let table = self
            .by_site_packages
            .entry(site_packages.clone())
            .or_insert_with(|| scan(&site_packages));
        table.get(top_level).cloned().unwrap_or_default()
    }
}

/// The directory that directly contains the top-level package or module.
fn site_packages_of(module_file: &Utf8Path, top_level: &str) -> Option<Utf8PathBuf> {
    let stem = module_file.file_stem()?;
    if stem == top_level {
        return module_file.parent().map(Utf8Path::to_path_buf);
    }
    module_file
        .ancestors()
        .find(|dir| dir.file_name() == Some(top_level))
        .and_then(Utf8Path::parent)
        .map(Utf8Path::to_path_buf)
}

/// Reads every `*.dist-info` under `site_packages` once.
fn scan(site_packages: &Utf8Path) -> BTreeMap<String, Vec<DistributionName>> {
    let mut table: BTreeMap<String, Vec<DistributionName>> = BTreeMap::new();
    let Ok(entries) = site_packages.read_dir_utf8() else {
        return table;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let Some(name) = dir.file_name().and_then(|n| n.strip_suffix(".dist-info")) else {
            continue;
        };
        let distribution = distribution_name(dir)
            .unwrap_or_else(|| DistributionName::normalize(name.split('-').next().unwrap_or(name)));
        for module in top_level_modules(dir) {
            let owners = table.entry(module).or_default();
            if !owners.contains(&distribution) {
                owners.push(distribution.clone());
            }
        }
    }
    table
}

/// `Name:` from `METADATA`.
fn distribution_name(dist_info: &Utf8Path) -> Option<DistributionName> {
    let metadata = std::fs::read_to_string(dist_info.join("METADATA")).ok()?;
    metadata
        .lines()
        .find_map(|line| line.strip_prefix("Name:"))
        .map(|name| DistributionName::normalize(name.trim()))
}

/// Top-level importable names from `top_level.txt`, else from `RECORD`.
fn top_level_modules(dist_info: &Utf8Path) -> Vec<String> {
    if let Ok(listed) = std::fs::read_to_string(dist_info.join("top_level.txt")) {
        let names: Vec<String> = listed
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect();
        if !names.is_empty() {
            return names;
        }
    }
    let Ok(record) = std::fs::read_to_string(dist_info.join("RECORD")) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for line in record.lines() {
        let Some(path) = line.split(',').next() else {
            continue;
        };
        let first = path.split('/').next().unwrap_or(path);
        let module = match first.strip_suffix(".py") {
            Some(stem) if !path.contains('/') => stem,
            _ if !first.contains('.') && first != "__pycache__" && !first.starts_with("..") => {
                first
            }
            _ => continue,
        };
        if !names.iter().any(|n| n == module) {
            names.push(module.to_owned());
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use camino::Utf8Path;

    use super::site_packages_of;

    #[test]
    fn finds_the_directory_holding_the_top_level_package_or_module() {
        assert_eq!(
            site_packages_of(
                Utf8Path::new("/venv/lib/site-packages/pydantic/fields.py"),
                "pydantic"
            )
            .unwrap(),
            "/venv/lib/site-packages"
        );
        assert_eq!(
            site_packages_of(Utf8Path::new("/venv/lib/site-packages/six.py"), "six").unwrap(),
            "/venv/lib/site-packages"
        );
    }
}
