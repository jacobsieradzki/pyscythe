//! SARIF 2.1.0 output, the format GitHub code scanning and most editors ingest.

use camino::Utf8Path;
use pyscythe_core::finding::{Confidence, Rule};
use pyscythe_core::report::Report;
use serde_json::{Value, json};

const SCHEMA: &str = "https://json.schemastore.org/sarif-2.1.0.json";

/// The whole SARIF log for one report.
pub(crate) fn document(report: &Report, root: &Utf8Path) -> Value {
    let rules: Vec<Value> = Rule::ALL
        .iter()
        .map(|rule| {
            json!({
                "id": rule.code(),
                "shortDescription": { "text": rule.description() },
                "helpUri": "https://github.com/jacobsieradzki/pyscythe",
            })
        })
        .collect();

    let results: Vec<Value> = report
        .findings
        .iter()
        .map(|finding| {
            let uri = finding
                .path
                .strip_prefix(root)
                .unwrap_or(&finding.path)
                .as_str();
            let mut region = serde_json::Map::new();
            if let Some(position) = finding.position {
                region.insert("startLine".into(), position.line.get().into());
                region.insert("startColumn".into(), position.column.get().into());
            }
            let level = match finding.confidence {
                Confidence::High | Confidence::Medium => "warning",
                Confidence::Low => "note",
            };
            json!({
                "ruleId": finding.rule.code(),
                "level": level,
                "message": { "text": finding.message },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": uri, "uriBaseId": "%SRCROOT%" },
                        "region": region,
                    }
                }],
                "properties": { "confidence": level_name(finding.confidence) },
            })
        })
        .collect();

    json!({
        "$schema": SCHEMA,
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "pyscythe",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": "https://github.com/jacobsieradzki/pyscythe",
                    "rules": rules,
                }
            },
            "originalUriBaseIds": {
                "%SRCROOT%": { "uri": format!("file://{}/", root.as_str().trim_end_matches('/')) }
            },
            "results": results,
        }]
    })
}

const fn level_name(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    }
}
