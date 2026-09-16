//! Acceptance tests: run the real binary against fixture projects.

use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn pyscythe() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pyscythe"))
}

#[test]
fn reports_unused_module_level_definitions_as_json() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("simple_unused"))
        .output()
        .expect("runs");

    assert_eq!(output.status.code(), Some(1), "findings exit with 1");

    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(report["kind"], "dead-code");
    assert_eq!(report["schema_version"], 1);

    let mut symbols: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .map(|f| f["symbol"].as_str().expect("symbol name"))
        .collect();
    symbols.sort_unstable();
    assert_eq!(symbols, ["UnusedThing", "unused_helper"]);

    let unused_helper = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .find(|f| f["symbol"] == "unused_helper")
        .expect("unused_helper finding");
    assert_eq!(unused_helper["rule"], "unused-function");
    assert_eq!(unused_helper["module"], "pkg.helpers");
    assert_eq!(unused_helper["position"]["line"], 5);
    assert_eq!(unused_helper["confidence"], "medium");
}

#[test]
fn human_format_lists_each_finding_with_its_location() {
    pyscythe()
        .arg("dead-code")
        .arg(fixture("simple_unused"))
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "helpers.py:5:5  function `unused_helper` is never used",
        ))
        .stdout(predicate::str::contains(
            "helpers.py:13:7  class `UnusedThing` is never used",
        ))
        .stdout(predicate::str::contains("2 finding(s)"));
}

#[test]
fn a_clean_project_exits_zero() {
    pyscythe()
        .arg("dead-code")
        .arg(fixture("all_used"))
        .assert()
        .success()
        .stdout(predicate::str::contains("No dead code found"));
}

#[test]
fn a_relative_path_is_resolved_against_the_working_directory() {
    pyscythe()
        .current_dir(fixture("simple_unused"))
        .args(["dead-code", "."])
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "function `unused_helper` is never used",
        ));
}

#[test]
fn a_missing_path_is_an_error() {
    pyscythe()
        .arg("dead-code")
        .arg(fixture("does_not_exist"))
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error:"));
}

#[test]
fn symbols_reached_through_aliased_imports_are_used() {
    pyscythe()
        .arg("dead-code")
        .arg(fixture("aliased_import"))
        .assert()
        .success()
        .stdout(predicate::str::contains("No dead code found"));
}

#[test]
fn framework_plugins_keep_conventional_roots() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("frameworks"))
        .output()
        .expect("runs");

    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let symbols: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .map(|f| f["symbol"].as_str().expect("symbol name"))
        .collect();
    assert_eq!(symbols, ["orphan"], "only the undecorated helper is dead");
    assert!(
        report["summary"]["symbols_kept"]
            .as_u64()
            .expect("kept count")
            >= 7,
        "routes, hook, commands, task, tests and entry point are kept: {report}"
    );
}

#[test]
fn plugins_can_be_switched_off() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json", "--no-plugins"])
        .arg(fixture("frameworks"))
        .output()
        .expect("runs");

    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let mut symbols: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter_map(|f| f["symbol"].as_str())
        .collect();
    symbols.sort_unstable();
    assert!(
        symbols.contains(&"create_user"),
        "route handler reported without plugins: {symbols:?}"
    );
    assert!(
        symbols.contains(&"main"),
        "entry point reported without plugins: {symbols:?}"
    );
    assert_eq!(report["summary"]["symbols_kept"], 0);
}

#[test]
fn orm_table_models_are_kept_but_plain_schemas_are_not() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("orm_models"))
        .output()
        .expect("runs");

    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let symbols: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .map(|f| f["symbol"].as_str().expect("symbol name"))
        .collect();
    assert_eq!(symbols, ["EventRead"]);
}

#[test]
fn notebooks_are_not_analysed() {
    pyscythe()
        .arg("dead-code")
        .arg(fixture("notebook"))
        .assert()
        .success()
        .stdout(predicate::str::contains("No dead code found in 1 files"));
}

#[test]
fn a_file_nobody_imports_is_reported_as_unused_and_scripts_are_not() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("unused_file"))
        .output()
        .expect("runs");

    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let findings = report["findings"].as_array().expect("findings array");
    assert_eq!(findings.len(), 1, "{report}");
    assert_eq!(findings[0]["rule"], "unused-file");
    assert_eq!(findings[0]["module"], "pkg.orphan");
    assert!(
        findings[0].get("symbol").is_none(),
        "file findings carry no symbol"
    );
}

#[test]
fn tool_pyscythe_config_excludes_ignores_and_adds_roots() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("configured"))
        .output()
        .expect("runs");

    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let symbols: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter_map(|f| f["symbol"].as_str())
        .collect();
    assert_eq!(symbols, ["still_dead"], "{report}");
    assert_eq!(report["summary"]["symbols_ignored"], 1);
    assert_eq!(
        report["summary"]["files_scanned"], 4,
        "scripts/ is excluded from reports"
    );
}

#[test]
fn dotted_strings_count_as_references() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("string_refs"))
        .output()
        .expect("runs");

    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let symbols: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter_map(|f| f["symbol"].as_str())
        .collect();
    // The string names the module `pkg.tasks`, which keeps the file, not the function in it.
    assert_eq!(symbols, ["UnusedMiddleware", "nightly"], "{report}");
    let unused_file_reported = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .any(|f| f["rule"] == "unused-file");
    assert!(
        !unused_file_reported,
        "pkg.tasks is named in a string: {report}"
    );
}

#[test]
fn unused_methods_are_reported_but_resolved_named_and_hooked_ones_are_not() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("methods"))
        .output()
        .expect("runs");

    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let mut findings: Vec<(&str, &str, &str)> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter_map(|f| {
            Some((
                f["owner"].as_str().unwrap_or(""),
                f["symbol"].as_str()?,
                f["confidence"].as_str()?,
            ))
        })
        .collect();
    findings.sort_unstable();
    assert_eq!(
        findings,
        [
            ("Base", "hook", "low"),
            ("TestWidget", "helper_nobody_calls", "low"),
            ("Widget", "_unused_private", "medium"),
            ("Widget", "unused_property", "low"),
            ("Widget", "unused_public", "low"),
        ],
        "{report}"
    );
    let kept: Vec<&str> = report["kept"]
        .as_array()
        .expect("kept array")
        .iter()
        .filter_map(|k| k["symbol"].as_str())
        .collect();
    assert_eq!(
        kept,
        ["hook", "TestWidget", "setup_method", "test_render"],
        "Child.hook overrides an inherited member: {report}"
    );
}

#[test]
fn show_kept_lists_plugin_decisions() {
    pyscythe()
        .args(["dead-code", "--show-kept"])
        .arg(fixture("frameworks"))
        .assert()
        .stdout(predicate::str::contains("Kept by plugins:"))
        .stdout(predicate::str::contains(
            "`create_user` kept by fastapi: registered as a route handler",
        ))
        .stdout(predicate::str::contains(
            "`main` kept by entry-points: declared as an entry point in pyproject.toml",
        ));
}

#[test]
fn exclude_flag_adds_to_configured_exclusions() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json", "--exclude", "pkg/lib.py"])
        .arg(fixture("configured"))
        .output()
        .expect("runs");

    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(report["summary"]["files_scanned"], 3, "{report}");
    assert!(
        report["findings"].as_array().expect("findings").is_empty(),
        "{report}"
    );
}

#[test]
fn a_decorator_resolved_to_a_local_lookalike_does_not_count_as_a_framework_hook() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("lookalike"))
        .output()
        .expect("runs");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let symbols: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter_map(|f| f["symbol"].as_str())
        .collect();
    assert_eq!(
        symbols,
        ["handler"],
        "router.get resolves to pkg.local_router, not fastapi: {report}"
    );
}

#[test]
fn libraries_keep_their_api_stubs_docs_and_fixtures_are_not_dead() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json", "--show-kept"])
        .arg(fixture("library"))
        .output()
        .expect("runs");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let symbols: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter_map(|f| f["symbol"].as_str())
        .collect();
    assert_eq!(symbols, ["_private_helper"], "{report}");
    let unused_file_reported = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .any(|f| f["rule"] == "unused-file");
    assert!(
        !unused_file_reported,
        "docs/conf.py and stubs are not unused files: {report}"
    );
    let kept: Vec<(&str, &str)> = report["kept"]
        .as_array()
        .expect("kept")
        .iter()
        .map(|k| {
            (
                k["symbol"].as_str().unwrap_or(""),
                k["why"].as_str().unwrap_or(""),
            )
        })
        .collect();
    assert!(kept.contains(&("connect", "listed in __all__")), "{kept:?}");
    assert!(
        kept.contains(&("helper", "public name of a module configured as API")),
        "{kept:?}"
    );
    // With pytest installed on the machine, ty binds the test's parameters to
    // their fixtures and both are simply used; without it, the plugin keeps
    // them by the `name=` keyword and the parameter name. Neither is dead.
    for (fixture, fallback) in [
        ("make_client", "fixture exposed under another name"),
        ("unnamed", "fixture requested by a test parameter"),
    ] {
        let reasons: Vec<&str> = kept
            .iter()
            .filter(|(name, _)| *name == fixture)
            .map(|(_, why)| *why)
            .collect();
        assert!(
            reasons.is_empty() || reasons == [fallback],
            "{fixture} is used or kept by the fallback, never dead: {kept:?}"
        );
    }
    assert_eq!(
        report["summary"]["files_scanned"], 4,
        "core.pyi is not analysed: {report}"
    );
}

#[test]
fn django_settings_modules_string_named_admin_fields_and_manager_hooks_are_kept() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("django_settings"))
        .output()
        .expect("runs");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let symbols: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter_map(|f| f["symbol"].as_str())
        .collect();
    assert_eq!(
        symbols,
        ["forgotten", "unused_line"],
        "summary_line is named in a template, money and badge are registered template tags: {report}"
    );
    let unused_file = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .any(|f| f["rule"] == "unused-file");
    assert!(
        !unused_file,
        "production.py is a settings variant: {report}"
    );
}

#[test]
fn classes_registered_by_an_init_subclass_hook_are_kept() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("registry"))
        .output()
        .expect("runs");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let symbols: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter_map(|f| f["symbol"].as_str())
        .collect();
    assert_eq!(
        symbols,
        ["Plain"],
        "Alpha is registered by Plugin.__init_subclass__: {report}"
    );
}

#[test]
fn registration_decorators_lower_confidence_and_http_handlers_are_kept() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("decorated"))
        .output()
        .expect("runs");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let findings: Vec<(&str, &str)> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter_map(|f| Some((f["symbol"].as_str()?, f["confidence"].as_str()?)))
        .collect();
    assert_eq!(
        findings,
        [("get_headers", "low"), ("helper", "low"), ("visit", "low")],
        "do_GET is dispatched by BaseHTTPRequestHandler, visit_Name by an f-string prefix: {report}"
    );
}

#[test]
fn pytest_collection_settings_decide_what_counts_as_a_test() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("pytest_config"))
        .output()
        .expect("runs");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let symbols: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter_map(|f| f["symbol"].as_str())
        .collect();
    assert_eq!(
        symbols,
        ["helper", "test_stale"],
        "python_files = check_*.py collects check_thing.py, not test_stale.py: {report}"
    );
}

#[test]
fn home_assistant_integrations_keep_their_hooks_constants_and_flow_steps() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("homeassistant"))
        .output()
        .expect("runs");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let findings: Vec<(&str, &str)> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter_map(|f| Some((f["rule"].as_str()?, f["symbol"].as_str().unwrap_or(""))))
        .collect();
    assert_eq!(
        findings,
        [("unused-function", "unused_helper")],
        "platform modules load by name, hooks and steps by convention: {report}"
    );
}

#[test]
fn code_behind_platform_checks_is_reachable_on_every_platform() {
    // ty would otherwise assume the host platform, and a Windows-only import would be dead
    // on macOS and alive on Windows. The same code must produce the same report everywhere.
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("platforms"))
        .output()
        .expect("runs");

    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn a_projects_ty_include_scope_does_not_hide_files_from_the_analysis() {
    // `[tool.ty.src] include` narrows what a project type-checks; everything is still code.
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("ty_scoped"))
        .output()
        .expect("runs");

    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(report["summary"]["files_scanned"], 4);
    let symbols: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter_map(|f| f["symbol"].as_str())
        .collect();
    assert_eq!(symbols, ["forgotten"]);
}

#[test]
fn graphql_schema_hooks_are_called_by_the_executor_not_by_name() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("graphene"))
        .output()
        .expect("runs");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let mut symbols: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter_map(|f| f["symbol"].as_str())
        .collect();
    symbols.sort_unstable();
    assert_eq!(
        symbols,
        ["helper", "resolve_country"],
        "only the hooks of schema types are kept: {report}"
    );
}

#[test]
fn classes_named_for_tests_are_collected_whatever_the_suffix() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("test_suffix"))
        .output()
        .expect("runs");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let mut symbols: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter_map(|f| f["symbol"].as_str())
        .collect();
    symbols.sort_unstable();
    assert_eq!(
        symbols,
        ["Helper", "build"],
        "`*Test` and `*Tests` are collected; a class with neither is not: {report}"
    );
}

#[test]
fn files_a_type_checker_reads_are_not_dead_but_a_module_named_typing_is_ordinary() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("typing_tests"))
        .output()
        .expect("runs");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let reported: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter_map(|f| f["module"].as_str())
        .collect();
    assert_eq!(
        reported,
        ["pkg.typing"],
        "typing tests are checked rather than called; `pkg/typing.py` is source: {report}"
    );
}

#[test]
fn integration_harness_targets_are_data_but_real_test_modules_are_not() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("harness_targets"))
        .output()
        .expect("runs");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let reported: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter_map(|f| f["symbol"].as_str())
        .collect();
    assert_eq!(
        reported,
        ["forgotten_helper"],
        "the harness copies targets and support trees into place: {report}"
    );
}

#[test]
fn vendored_third_party_code_is_not_this_projects_dead_code() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("vendored"))
        .output()
        .expect("runs");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let reported: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter_map(|f| f["symbol"].as_str())
        .collect();
    assert_eq!(
        reported,
        ["stop"],
        "vendored trees are carried verbatim; only our own code is ours: {report}"
    );
}

#[test]
fn textual_dispatches_messages_to_handlers_it_names_at_runtime() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("textual_app"))
        .output()
        .expect("runs");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let mut reported: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter_map(|f| f["symbol"].as_str())
        .collect();
    reported.sort_unstable();
    assert_eq!(
        reported,
        ["_on_mount", "helper"],
        "handlers on a message pump are kept; the same name elsewhere is not: {report}"
    );
}

#[test]
fn a_projects_own_test_base_collects_its_subclasses_and_test_data_is_input() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("test_base"))
        .output()
        .expect("runs");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let mut reported: Vec<String> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .map(|f| {
            let name = f["symbol"].as_str().unwrap_or("-");
            format!("{}:{name}", f["rule"].as_str().unwrap_or("?"))
        })
        .collect();
    reported.sort();
    assert_eq!(
        reported,
        ["unused-class:Detached", "unused-method:helper"],
        "a base under a testing package collects, and test-data is input: {report}"
    );
}

#[test]
fn ansible_plugins_and_modules_are_found_by_the_plugin_loader() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("ansible_plugins"))
        .output()
        .expect("runs");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let mut reported: Vec<String> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .map(|f| {
            let name = f["symbol"].as_str().unwrap_or("-");
            format!("{}:{name}", f["rule"].as_str().unwrap_or("?"))
        })
        .collect();
    reported.sort();
    assert_eq!(
        reported,
        ["unused-function:forgotten_helper"],
        "plugin and module trees are loaded by name; utils is ordinary code: {report}"
    );
}

#[test]
fn a_module_that_reads_its_own_globals_uses_every_name_it_defines() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("reflective_globals"))
        .output()
        .expect("runs");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let reported: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter_map(|f| f["symbol"].as_str())
        .collect();
    assert_eq!(
        reported,
        ["FORGOTTEN"],
        "the token table is built from globals(); the plain module is ordinary: {report}"
    );
}

#[test]
fn reports_attributes_nothing_reads_but_not_the_fields_of_a_data_class() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json"])
        .arg(fixture("attributes"))
        .output()
        .expect("runs");

    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let mut attributes: Vec<(&str, &str, &str)> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| f["rule"] == "unused-attribute")
        .filter_map(|f| {
            Some((
                f["owner"].as_str().unwrap_or(""),
                f["symbol"].as_str()?,
                f["confidence"].as_str()?,
            ))
        })
        .collect();
    attributes.sort_unstable();
    assert_eq!(
        attributes,
        [
            ("Settings", "_cache_size", "medium"),
            ("Settings", "_pool", "medium"),
            ("Settings", "retries", "low"),
        ],
        "{report}"
    );
}

#[test]
fn entry_points_declared_in_setup_py_and_setup_cfg_keep_their_targets() {
    let output = pyscythe()
        .args(["dead-code", "--format", "json", "--show-kept"])
        .arg(fixture("legacy_entry_points"))
        .output()
        .expect("runs");

    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let mut kept: Vec<&str> = report["kept"]
        .as_array()
        .expect("kept array")
        .iter()
        .filter(|k| k["plugin"] == "entry-points")
        .filter_map(|k| k["symbol"].as_str())
        .collect();
    kept.sort_unstable();
    assert_eq!(kept, ["launch", "main"], "{report}");

    let mut reported: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter_map(|f| f["symbol"].as_str().or_else(|| f["module"].as_str()))
        .collect();
    reported.sort_unstable();
    assert_eq!(reported, ["forgotten", "pkg.orphan"], "{report}");
}
