//! Which installed distribution provides which top-level modules, read from
//! the `RECORD` and `top_level.txt` files in `*.dist-info` directories.

use std::collections::BTreeMap;

use camino::{Utf8Path, Utf8PathBuf};
use pyscythe_core::manifest::DistributionName;

/// What one site-packages directory's metadata says.
#[derive(Debug, Default)]
struct SitePackages {
    /// Top-level module -> owning distributions.
    owners: BTreeMap<String, Vec<DistributionName>>,
    /// Distribution -> what its `Requires-Dist` lines name.
    requirements: BTreeMap<DistributionName, Vec<DistributionName>>,
}

/// Distribution metadata per site-packages directory, read on demand.
#[derive(Debug, Default)]
pub(crate) struct DistributionIndex {
    by_site_packages: BTreeMap<Utf8PathBuf, SitePackages>,
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
        table.owners.get(top_level).cloned().unwrap_or_default()
    }

    /// What `distribution` requires, from whichever scanned site-packages
    /// holds it; empty when none does.
    pub(crate) fn requirements_of(&self, distribution: &DistributionName) -> Vec<DistributionName> {
        self.by_site_packages
            .values()
            .find_map(|site| site.requirements.get(distribution).cloned())
            .unwrap_or_default()
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
fn scan(site_packages: &Utf8Path) -> SitePackages {
    let mut table = SitePackages::default();
    let Ok(entries) = site_packages.read_dir_utf8() else {
        return table;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let Some(name) = dir.file_name().and_then(|n| n.strip_suffix(".dist-info")) else {
            continue;
        };
        let metadata = std::fs::read_to_string(dir.join("METADATA")).unwrap_or_default();
        let distribution = distribution_name(&metadata)
            .unwrap_or_else(|| DistributionName::normalize(name.split('-').next().unwrap_or(name)));
        for module in top_level_modules(dir) {
            let owners = table.owners.entry(module).or_default();
            if !owners.contains(&distribution) {
                owners.push(distribution.clone());
            }
        }
        table
            .requirements
            .insert(distribution, requires_dist(&metadata));
    }
    table
}

/// `Name:` from `METADATA` text.
fn distribution_name(metadata: &str) -> Option<DistributionName> {
    metadata
        .lines()
        .find_map(|line| line.strip_prefix("Name:"))
        .map(|name| DistributionName::normalize(name.trim()))
}

/// The distributions named by `Requires-Dist:` lines, extras included: an
/// extra a project asked for is as installed as anything else.
fn requires_dist(metadata: &str) -> Vec<DistributionName> {
    let mut names: Vec<DistributionName> = Vec::new();
    for requirement in metadata
        .lines()
        .filter_map(|line| line.strip_prefix("Requires-Dist:"))
    {
        let name: String = requirement
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            .collect();
        if name.is_empty() {
            continue;
        }
        let name = DistributionName::normalize(&name);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
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

    use super::{requires_dist, site_packages_of};

    #[test]
    fn reads_requirement_names_from_metadata() {
        let metadata = "Name: fastapi\nRequires-Dist: starlette (>=0.40.0,<0.47.0)\nRequires-Dist: pydantic>=1.7.4,!=1.8\nRequires-Dist: httpx>=0.23.0; extra == \"all\"\nRequires-Dist: starlette\n";
        let names: Vec<String> = requires_dist(metadata)
            .iter()
            .map(|n| n.as_str().to_owned())
            .collect();
        assert_eq!(names, ["starlette", "pydantic", "httpx"]);
    }

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
