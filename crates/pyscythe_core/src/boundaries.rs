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
    fn deny_rules_apply_on_top_of_layers() {
        let mut index = FakeIndex::new();
        let domain = index.add_file("/p/app/domain/order.py", "app.domain.order");
        let infra = index.add_file("/p/app/infra/db.py", "app.infra.db");
        index.add_import(domain, infra, ImportKind::Runtime);
        let config = BoundaryConfig {
            rules: vec![DenyRule {
                from: ModulePrefix::new("app.domain"),
                deny: vec![ModulePrefix::new("app.infra")],
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
