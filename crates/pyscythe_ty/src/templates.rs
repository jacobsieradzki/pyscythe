//! Django and Jinja templates name methods and properties in `{{ }}` and
//! `{% %}` blocks; nothing in Python refers to `fieldset.is_collapsible`, the
//! template does. Every identifier inside such a block counts as an attribute
//! name in use, the same safety net `getattr` and string literals get.

use camino::Utf8Path;
use rustc_hash::FxHashSet;

/// Directories never worth descending into.
const SKIP: &[&str] = &[
    ".venv",
    "venv",
    "node_modules",
    "target",
    "build",
    "dist",
    "site-packages",
    "__pycache__",
];

/// Template file extensions.
const EXTENSIONS: &[&str] = &["html", "htm", "txt", "xml", "jinja", "jinja2", "j2"];

/// Templates larger than this are generated or vendored, not hand-written.
const MAX_BYTES: u64 = 1 << 20;

/// Every identifier used inside a template block, over every template file
/// under a `templates` directory beneath `root`.
pub(crate) fn attribute_names(root: &Utf8Path) -> FxHashSet<String> {
    let mut names = FxHashSet::default();
    let mut pending = vec![(root.to_path_buf(), false)];
    while let Some((dir, in_templates)) = pending.pop() {
        let Ok(entries) = dir.read_dir_utf8() else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap_or_default();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                if !name.starts_with('.') && !SKIP.contains(&name) {
                    pending.push((path.to_path_buf(), in_templates || name == "templates"));
                }
                continue;
            }
            let is_template = in_templates
                && path
                    .extension()
                    .is_some_and(|extension| EXTENSIONS.contains(&extension));
            if !is_template || entry.metadata().is_ok_and(|m| m.len() > MAX_BYTES) {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(path) {
                collect_block_identifiers(&text, &mut names);
            }
        }
    }
    names
}

/// Identifiers inside `{{ ... }}` and `{% ... %}` in `text`.
fn collect_block_identifiers(text: &str, names: &mut FxHashSet<String>) {
    let mut rest = text;
    while let Some(open) = rest.find(['{']) {
        let Some(after) = rest.get(open + 1..) else {
            break;
        };
        let closer = match after.chars().next() {
            Some('{') => "}}",
            Some('%') => "%}",
            _ => {
                rest = after;
                continue;
            }
        };
        let Some(body) = after.get(1..) else {
            break;
        };
        let Some(end) = body.find(closer) else {
            break;
        };
        let block = body.get(..end).unwrap_or_default();
        for token in block.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
            if token
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            {
                names.insert(token.to_owned());
            }
        }
        rest = body.get(end + closer.len()..).unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    use rustc_hash::FxHashSet;

    use super::collect_block_identifiers;

    #[test]
    fn reads_names_out_of_variable_and_tag_blocks_only() {
        let mut names = FxHashSet::default();
        collect_block_identifiers(
            "<h1>{{ fieldset.name|title }}</h1>{% if fieldset.is_collapsible %}<p>plain.text</p>{% endif %}",
            &mut names,
        );
        assert!(names.contains("is_collapsible"));
        assert!(names.contains("title"));
        assert!(names.contains("endif"));
        assert!(!names.contains("plain"), "text outside blocks is prose");
        assert!(!names.contains("h1"));
    }
}
