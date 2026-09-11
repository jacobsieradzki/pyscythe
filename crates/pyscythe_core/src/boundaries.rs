//! Checks the import graph against configured architecture boundaries.

use crate::config::{BoundaryConfig, ModulePrefix, TypeOnlyImports};
use crate::finding::{Confidence, Detail, Finding, Rule};
use crate::index::{CodebaseIndex, ImportKind};
use crate::report::{Report, ReportKind, Summary};

/// Runs the boundary analysis over every import in `index`.
#[must_use]
pub fn analyze(index: &dyn CodebaseIndex, config: &BoundaryConfig) -> Report {
    let files = index.files();
    let mut findings = Vec::new();

    for file in files {
        let Some(from_module) = &file.module else {
            continue;
        };
        for import in index.imports(file.id) {
            if import.kind == ImportKind::TypeOnly && config.type_only == TypeOnlyImports::Allow {
                continue;
            }
            let Some(target) = index.file(import.target) else {
                continue;
            };
            let Some(to_module) = &target.module else {
                continue;
            };
            if to_module == from_module {
                continue;
            }
            let Some(rule_text) = violation(config, from_module.as_str(), to_module.as_str())
            else {
                continue;
            };
            findings.push(Finding {
                rule: Rule::BoundaryViolation,
                path: file.path.clone(),
                module: file.module.clone(),
                position: index.position(file.id, import.span.start()),
                confidence: Confidence::High,
                message: format!(
                    "`{}` imports `{}`: {rule_text}",
                    from_module.as_str(),
                    to_module.as_str()
                ),
                detail: Detail::Import {
                    from_module: from_module.clone(),
                    to_module: to_module.clone(),
                    rule_text,
                },
            });
        }
    }

    // Importing `a.b.c` also imports `a.b` and `a`; report each statement once,
    // against the most specific module it names.
    findings.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.position.cmp(&b.position))
            .then(target_depth(b).cmp(&target_depth(a)))
    });
    findings.dedup_by(|later, earlier| {
        later.path == earlier.path && later.position == earlier.position
    });

    Report {
        schema_version: Report::SCHEMA_VERSION,
        kind: ReportKind::Boundaries,
        summary: Summary {
            files_scanned: files.len(),
            symbols_checked: 0,
            symbols_kept: 0,
            symbols_ignored: 0,
            suppressed: 0,
            baselined: 0,
            findings: findings.len(),
            changed_files: None,
            health: None,
            duplication: None,
        },
        findings,
        kept: Vec::new(),
    }
}

fn target_depth(finding: &Finding) -> usize {
    match &finding.detail {
        Detail::Import { to_module, .. } => to_module.as_str().matches('.').count(),
        _ => 0,
    }
}

/// A layering proposal derived from the import graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// Ranks from top to bottom; each rank lists the second-level packages in it.
    pub layers: Vec<Vec<ModulePrefix>>,
    /// Packages that import each other and so cannot be layered until untangled.
    pub tangles: Vec<Vec<ModulePrefix>>,
}

impl Suggestion {
    /// The proposal as a `[tool.pyscythe.boundaries]` table.
    #[must_use]
    pub fn to_toml(&self) -> String {
        let mut out = String::from("[tool.pyscythe.boundaries]\nlayers = [\n");
        for rank in &self.layers {
            let names: Vec<String> = rank.iter().map(|p| format!("\"{}\"", p.as_str())).collect();
            out.push_str(&format!("    [{}],\n", names.join(", ")));
        }
        out.push_str("]\n");
        for tangle in &self.tangles {
            let names: Vec<&str> = tangle.iter().map(ModulePrefix::as_str).collect();
            out.push_str(&format!(
                "# {} import each other; they share a rank until the cycle is broken.\n",
                names.join(", ")
            ));
        }
        out
    }
}

/// Proposes layers from how second-level packages (`app.api`, `app.domain`)
/// import each other at load time: importers rank above what they import.
///
/// Packages in an import cycle are condensed into one rank and reported as a
/// tangle. Modules with fewer than two segments have no package to rank.
#[must_use]
pub fn suggest(index: &dyn CodebaseIndex) -> Suggestion {
    let files = index.files();
    let package_of = |file: &crate::source::SourceFile| -> Option<ModulePrefix> {
        // Tests, docs, examples, and entry scripts are consumers of the
        // architecture, not layers in it.
        if crate::dead_code::is_in_root_directory(file)
            || crate::dead_code::is_test_file(file.file_name())
            || matches!(file.file_name(), "__main__.py" | "conftest.py")
            || file
                .relative_path
                .components()
                .any(|c| matches!(c.as_str(), "tests" | "test"))
        {
            return None;
        }
        let module = file.module.as_ref()?;
        let mut segments = module.as_str().split('.');
        let (first, second) = (segments.next()?, segments.next()?);
        Some(ModulePrefix::new(format!("{first}.{second}")))
    };

    let mut packages: Vec<ModulePrefix> = files.iter().filter_map(package_of).collect();
    packages.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    packages.dedup();
    let position = |prefix: &ModulePrefix| packages.iter().position(|p| p == prefix);

    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); packages.len()];
    for file in files {
        let (Some(from), Some(from_index)) = (
            package_of(file),
            package_of(file).and_then(|p| position(&p)),
        ) else {
            continue;
        };
        for import in index.imports(file.id) {
            if import.kind != ImportKind::Runtime {
                continue;
            }
            let Some(to) = index.file(import.target).and_then(package_of) else {
                continue;
            };
            if to == from {
                continue;
            }
            if let Some(to_index) = position(&to)
                && !edges[from_index].contains(&to_index)
            {
                edges[from_index].push(to_index);
            }
        }
    }

    let components = crate::graph::strongly_connected_components(&edges);
    let component_of: Vec<usize> = {
        let mut lookup = vec![0; packages.len()];
        for (index, component) in components.iter().enumerate() {
            for &node in component {
                lookup[node] = index;
            }
        }
        lookup
    };
    // Condensed DAG: rank = longest chain of importers above.
    let mut condensed: Vec<Vec<usize>> = vec![Vec::new(); components.len()];
    for (from, targets) in edges.iter().enumerate() {
        for &to in targets {
            let (a, b) = (component_of[from], component_of[to]);
            if a != b && !condensed[a].contains(&b) {
                condensed[a].push(b);
            }
        }
    }
    let mut rank = vec![0usize; components.len()];
    for _ in 0..components.len() {
        for (from, targets) in condensed.iter().enumerate() {
            for &to in targets {
                if rank[to] < rank[from] + 1 {
                    rank[to] = rank[from] + 1;
                }
            }
        }
    }

    let depth = rank.iter().copied().max().map_or(0, |max| max + 1);
    let mut layers: Vec<Vec<ModulePrefix>> = vec![Vec::new(); depth];
    let mut tangles = Vec::new();
    for (index, component) in components.iter().enumerate() {
        let members: Vec<ModulePrefix> = component.iter().map(|&n| packages[n].clone()).collect();
        if members.len() > 1 {
            tangles.push(members.clone());
        }
        layers[rank[index]].extend(members);
    }
    for layer in &mut layers {
        layer.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    }
    layers.retain(|layer| !layer.is_empty());
    Suggestion { layers, tangles }
}

/// Why `from` may not import `to`, if it may not.
fn violation(config: &BoundaryConfig, from: &str, to: &str) -> Option<String> {
    if let (Some(from_rank), Some(to_rank)) = (config.rank_of(from), config.rank_of(to))
        && to_rank < from_rank
    {
        let name = |rank: usize| {
            config
                .layers
                .get(rank)
                .map(|prefixes| {
                    prefixes
                        .iter()
                        .map(ModulePrefix::as_str)
                        .collect::<Vec<_>>()
                        .join("|")
                })
                .unwrap_or_default()
        };
        return Some(format!(
            "layer `{}` may not depend on the layer above it, `{}`",
            name(from_rank),
            name(to_rank)
        ));
    }
    config
        .rules
        .iter()
        .filter(|rule| rule.from.covers(from))
        .filter(|rule| !rule.allow.iter().any(|allowed| allowed.covers(to)))
        .find_map(|rule| {
            rule.deny
                .iter()
                .find(|denied| denied.covers(to))
                .map(|denied| {
                    format!(
                        "`{}` may not import `{}`",
                        rule.from.as_str(),
                        denied.as_str()
                    )
                })
        })
}

#[cfg(test)]
mod tests {
    use super::analyze;
    use crate::config::{BoundaryConfig, DenyRule, ModulePrefix, TypeOnlyImports};
    use crate::index::ImportKind;
    use crate::testing::FakeIndex;

    fn layered() -> BoundaryConfig {
        BoundaryConfig {
            layers: vec![
                vec![ModulePrefix::new("app.api")],
                vec![ModulePrefix::new("app.services")],
                vec![ModulePrefix::new("app.domain")],
            ],
            rules: Vec::new(),
            type_only: TypeOnlyImports::Allow,
        }
    }

    #[test]
    fn importing_downwards_is_fine_and_upwards_is_not() {
        let mut index = FakeIndex::new();
        let api = index.add_file("/p/app/api/routes.py", "app.api.routes");
        let services = index.add_file("/p/app/services/orders.py", "app.services.orders");
        let domain = index.add_file("/p/app/domain/order.py", "app.domain.order");
        index.add_import(api, services, ImportKind::Runtime);
        index.add_import(services, domain, ImportKind::Runtime);
        index.add_import(domain, api, ImportKind::Runtime);

        let report = analyze(&index, &layered());

        assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
        assert_eq!(
            report.findings[0].message,
            "`app.domain.order` imports `app.api.routes`: layer `app.domain` may not depend on the layer above it, `app.api`"
        );
    }

    #[test]
    fn type_only_imports_are_allowed_unless_configured_otherwise() {
        let mut index = FakeIndex::new();
        let api = index.add_file("/p/app/api/routes.py", "app.api.routes");
        let domain = index.add_file("/p/app/domain/order.py", "app.domain.order");
        index.add_import(domain, api, ImportKind::TypeOnly);

        assert!(analyze(&index, &layered()).is_clean());
        let strict = BoundaryConfig {
            type_only: TypeOnlyImports::Check,
            ..layered()
        };
        assert_eq!(analyze(&index, &strict).findings.len(), 1);
    }

    #[test]
    fn modules_outside_every_layer_are_unconstrained() {
        let mut index = FakeIndex::new();
        let cli = index.add_file("/p/app/cli.py", "app.cli");
        let api = index.add_file("/p/app/api/routes.py", "app.api.routes");
        let domain = index.add_file("/p/app/domain/order.py", "app.domain.order");
        index.add_import(cli, api, ImportKind::Runtime);
        index.add_import(domain, cli, ImportKind::Runtime);

        assert!(analyze(&index, &layered()).is_clean());
    }

    #[test]
    fn allow_lists_carve_exceptions_out_of_deny_rules() {
        let mut index = FakeIndex::new();
        let domain = index.add_file("/p/app/domain/order.py", "app.domain.order");
        let types = index.add_file("/p/app/infra/types.py", "app.infra.types");
        let db = index.add_file("/p/app/infra/db.py", "app.infra.db");
        index.add_import(domain, types, ImportKind::Runtime);
        index.add_import(domain, db, ImportKind::Runtime);
        let config = BoundaryConfig {
            rules: vec![DenyRule {
                from: ModulePrefix::new("app.domain"),
                deny: vec![ModulePrefix::new("app.infra")],
                allow: vec![ModulePrefix::new("app.infra.types")],
            }],
            ..layered()
        };

        let report = analyze(&index, &config);

        assert_eq!(report.findings.len(), 1);
        assert!(report.findings[0].message.contains("`app.infra.db`"));
    }

    #[test]
    fn suggests_layers_from_who_imports_whom() {
        let mut index = FakeIndex::new();
        let api = index.add_file("/p/app/api/routes.py", "app.api.routes");
        let services = index.add_file("/p/app/services/orders.py", "app.services.orders");
        let workers = index.add_file("/p/app/workers/jobs.py", "app.workers.jobs");
        let domain = index.add_file("/p/app/domain/order.py", "app.domain.order");
        index.add_import(api, services, ImportKind::Runtime);
        index.add_import(services, domain, ImportKind::Runtime);
        index.add_import(workers, domain, ImportKind::Runtime);
        index.add_import(api, workers, ImportKind::Runtime);

        let suggestion = super::suggest(&index);

        let names: Vec<Vec<&str>> = suggestion
            .layers
            .iter()
            .map(|rank| rank.iter().map(ModulePrefix::as_str).collect())
            .collect();
        assert_eq!(
            names,
            [
                vec!["app.api"],
                vec!["app.services", "app.workers"],
                vec!["app.domain"]
            ]
        );
        assert!(suggestion.tangles.is_empty());
        assert!(
            suggestion
                .to_toml()
                .contains("[\"app.services\", \"app.workers\"],")
        );
    }

    #[test]
    fn tests_docs_and_entry_scripts_do_not_become_layers() {
        let mut index = FakeIndex::new();
        let api = index.add_file("/p/app/api/routes.py", "app.api.routes");
        let test = index.add_file("/p/tests/test_api.py", "tests.test_api");
        let main = index.add_file("/p/app/__main__.py", "app.__main__");
        let docs = index.add_file("/p/docs/conf.py", "docs.conf");
        index.add_import(test, api, ImportKind::Runtime);
        index.add_import(main, api, ImportKind::Runtime);
        index.add_import(docs, api, ImportKind::Runtime);

        let suggestion = super::suggest(&index);

        assert_eq!(suggestion.layers.len(), 1);
        assert_eq!(suggestion.layers[0][0].as_str(), "app.api");
    }

    #[test]
    fn packages_that_import_each_other_share_a_rank_and_are_called_out() {
        let mut index = FakeIndex::new();
        let api = index.add_file("/p/app/api/routes.py", "app.api.routes");
        let services = index.add_file("/p/app/services/orders.py", "app.services.orders");
        let domain = index.add_file("/p/app/domain/order.py", "app.domain.order");
        index.add_import(api, services, ImportKind::Runtime);
        index.add_import(services, api, ImportKind::Runtime);
        index.add_import(services, domain, ImportKind::Runtime);

        let suggestion = super::suggest(&index);

        assert_eq!(suggestion.layers.len(), 2);
        assert_eq!(suggestion.tangles.len(), 1);
        assert!(suggestion.to_toml().contains("import each other"));
    }

    #[test]
    fn deny_rules_apply_on_top_of_layers() {
        let mut index = FakeIndex::new();
        let domain = index.add_file("/p/app/domain/order.py", "app.domain.order");
        let infra = index.add_file("/p/app/infra/db.py", "app.infra.db");
        index.add_import(domain, infra, ImportKind::Runtime);
        let config = BoundaryConfig {
            rules: vec![DenyRule {
                from: ModulePrefix::new("app.domain"),
                deny: vec![ModulePrefix::new("app.infra")],
                allow: Vec::new(),
            }],
            ..layered()
        };

        let report = analyze(&index, &config);

        assert_eq!(report.findings.len(), 1);
        assert!(
            report.findings[0]
                .message
                .ends_with("`app.domain` may not import `app.infra`")
        );
    }
}
