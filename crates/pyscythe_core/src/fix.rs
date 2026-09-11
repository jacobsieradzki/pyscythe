//! Turns dead-code findings into file edits and deletions.

use camino::Utf8PathBuf;

use crate::edit::{BodyAfterRemoval, Deletable};
use crate::finding::{Detail, Finding, Rule};
use crate::index::CodebaseIndex;
use crate::report::Report;
use crate::source::{ByteSpan, FileId};
use crate::symbol::SymbolName;

/// A rewritten file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEdit {
    /// The file.
    pub path: Utf8PathBuf,
    /// Its text before.
    pub before: String,
    /// Its text after.
    pub after: String,
    /// The definitions removed, in source order.
    pub removed: Vec<SymbolName>,
}

/// A finding the fixer would not act on, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// The finding.
    pub finding: Finding,
    /// A short reason.
    pub reason: &'static str,
}

/// Everything the fixer would do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FixPlan {
    /// Files to rewrite.
    pub edits: Vec<FileEdit>,
    /// Files to delete outright.
    pub deletions: Vec<Utf8PathBuf>,
    /// Findings left alone.
    pub skipped: Vec<Skipped>,
}

impl FixPlan {
    /// Whether the plan changes anything.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.edits.is_empty() && self.deletions.is_empty()
    }

    /// Definitions removed across all edits.
    #[must_use]
    pub fn removed_definitions(&self) -> usize {
        self.edits.iter().map(|e| e.removed.len()).sum()
    }
}

/// Plans deletions for the symbol and file findings in `report`.
///
/// Symbol findings are matched to deletable definitions by name and
/// position; a finding with no whole-line definition at that spot, such as a
/// `def` nested inside an `if`, is skipped, as is a method whose removal
/// would leave its class body empty.
#[must_use]
pub fn plan(index: &dyn CodebaseIndex, report: &Report) -> FixPlan {
    let mut plan = FixPlan::default();

    for file in index.files() {
        let findings: Vec<&Finding> = report
            .findings
            .iter()
            .filter(|f| f.path == file.path)
            .collect();
        if findings.is_empty() {
            continue;
        }
        if findings.iter().any(|f| f.rule == Rule::UnusedFile) {
            plan.deletions.push(file.path.clone());
            continue;
        }
        plan_file(index, file.id, &findings, &mut plan);
    }

    for finding in &report.findings {
        let handled = plan.deletions.contains(&finding.path)
            || plan.edits.iter().any(|e| e.path == finding.path)
            || plan.skipped.iter().any(|s| s.finding == *finding);
        if !handled {
            plan.skipped.push(Skipped {
                finding: finding.clone(),
                reason: "not a removable definition",
            });
        }
    }
    plan
}

fn plan_file(index: &dyn CodebaseIndex, file: FileId, findings: &[&Finding], plan: &mut FixPlan) {
    let Some(source) = index.source(file) else {
        return;
    };
    let deletables = index.deletables(file);

    let mut chosen: Vec<(Deletable, &Finding)> = Vec::new();
    for finding in findings {
        let Detail::Symbol { symbol, .. } = &finding.detail else {
            plan.skipped.push(Skipped {
                finding: (*finding).clone(),
                reason: "not a definition",
            });
            continue;
        };
        let Some(position) = finding.position else {
            continue;
        };
        let Some(deletable) = deletables
            .iter()
            .find(|d| d.name == *symbol && d.line == position.line && d.column == position.column)
        else {
            plan.skipped.push(Skipped {
                finding: (*finding).clone(),
                reason: "definition is not at module or class level",
            });
            continue;
        };
        if deletable.body_after_removal == BodyAfterRemoval::WouldBeEmpty {
            plan.skipped.push(Skipped {
                finding: (*finding).clone(),
                reason: "removing it would leave the class body empty",
            });
            continue;
        }
        chosen.push((deletable.clone(), finding));
    }

    // Outer definitions swallow inner ones: a method inside a deleted class needs no edit of its own.
    chosen.sort_by_key(|(d, _)| d.lines.start());
    let mut kept: Vec<Deletable> = Vec::new();
    for (deletable, _) in chosen {
        if kept
            .last()
            .is_some_and(|outer| outer.lines.encloses(deletable.lines))
        {
            continue;
        }
        kept.push(deletable);
    }
    if kept.is_empty() {
        return;
    }

    let mut after = source.clone();
    for deletable in kept.iter().rev() {
        after = remove_block(&after, deletable.lines);
    }
    plan.edits.push(FileEdit {
        path: index.file(file).map(|f| f.path.clone()).unwrap_or_default(),
        before: source,
        after,
        removed: kept.into_iter().map(|d| d.name).collect(),
    });
}

/// Removes `lines` from `text`, then trims blank lines at the join so the
/// remaining neighbours keep the larger of the two gaps that surrounded the block.
fn remove_block(text: &str, lines: ByteSpan) -> String {
    let start = lines.start().get() as usize;
    let end = lines.end().get() as usize;
    let (Some(head), Some(tail)) = (text.get(..start), text.get(end..)) else {
        return text.to_owned();
    };
    if tail.trim().is_empty() {
        // The block was last in the file: nothing follows, so drop the gap before it too.
        let trimmed = head.trim_end_matches(['\n', ' ', '\t']);
        return if trimmed.is_empty() {
            String::new()
        } else {
            format!("{trimmed}\n")
        };
    }
    let preceding = trailing_blank_lines(head);
    let following = leading_blank_lines(tail);
    let mut tail = tail;
    for _ in 0..preceding.min(following) {
        tail = tail.split_once('\n').map_or("", |(_, rest)| rest);
    }
    format!("{head}{tail}")
}

fn trailing_blank_lines(text: &str) -> usize {
    text.rsplit_terminator('\n')
        .take_while(|line| line.trim().is_empty())
        .count()
}

fn leading_blank_lines(text: &str) -> usize {
    text.split('\n')
        .take_while(|line| line.trim().is_empty())
        .count()
        .min(text.matches('\n').count())
}

#[cfg(test)]
mod tests {
    use super::{plan, remove_block};
    use crate::config::Config;
    use crate::dead_code;
    use crate::keep::Policy;
    use crate::manifest::Manifest;
    use crate::source::{ByteOffset, ByteSpan};
    use crate::symbol::SymbolKind;
    use crate::testing::FakeIndex;

    #[test]
    fn removing_a_block_keeps_the_surrounding_gap() {
        let text = "a = 1\n\n\ndef gone():\n    pass\n\n\nb = 2\n";
        let start = text.find("def").unwrap();
        let end = text.find("b = 2").unwrap() - 2;
        let after = remove_block(
            text,
            ByteSpan::new(
                ByteOffset::new(u32::try_from(start).unwrap()),
                ByteOffset::new(u32::try_from(end).unwrap()),
            ),
        );
        assert_eq!(after, "a = 1\n\n\nb = 2\n");
    }

    #[test]
    fn removing_the_last_block_leaves_no_trailing_blank_lines() {
        let text = "a = 1\n\n\ndef gone():\n    pass\n";
        let start = text.find("def").unwrap();
        let after = remove_block(
            text,
            ByteSpan::new(
                ByteOffset::new(u32::try_from(start).unwrap()),
                ByteOffset::new(u32::try_from(text.len()).unwrap()),
            ),
        );
        assert_eq!(after, "a = 1\n");
    }

    #[test]
    fn plans_edits_for_symbol_findings_and_deletions_for_file_findings() {
        let mut index = FakeIndex::new();
        let live = index.add_file("/proj/pkg/live.py", "pkg.live");
        let orphan = index.add_file("/proj/pkg/orphan.py", "pkg.orphan");
        let main = index.add_file("/proj/pkg/__main__.py", "pkg.__main__");
        index.add_import(main, live, crate::index::ImportKind::Runtime);
        let used = index.add_symbol(live, "used", SymbolKind::Function);
        index.add_reference(used, main);
        index.add_symbol(live, "dead", SymbolKind::Function);
        index.add_symbol(orphan, "lonely", SymbolKind::Function);
        index
            .set_source_with_deletables(live, "def used():\n    pass\n\n\ndef dead():\n    pass\n");

        let report = dead_code::analyze(
            &index,
            &Policy::none(),
            &Manifest::empty(),
            &Config::default(),
        );
        let plan = plan(&index, &report);

        assert_eq!(
            plan.deletions,
            [camino::Utf8PathBuf::from("/proj/pkg/orphan.py")]
        );
        assert_eq!(plan.edits.len(), 1);
        assert_eq!(plan.edits[0].after, "def used():\n    pass\n");
        assert_eq!(plan.edits[0].removed[0].as_str(), "dead");
        assert!(plan.skipped.is_empty(), "{:?}", plan.skipped);
    }

    #[test]
    fn a_method_that_would_empty_its_class_is_skipped() {
        let mut index = FakeIndex::new();
        let file = index.add_file("/proj/pkg/m.py", "pkg.m");
        let main = index.add_file("/proj/pkg/__main__.py", "pkg.__main__");
        index.add_import(main, file, crate::index::ImportKind::Runtime);
        let class = index.add_symbol(file, "Widget", SymbolKind::Class);
        index.add_reference(class, main);
        index.add_nested_symbol(file, class, "only", SymbolKind::Method);
        index
            .set_source_with_deletables(file, "class Widget:\n    def only(self):\n        pass\n");

        let report = dead_code::analyze(
            &index,
            &Policy::none(),
            &Manifest::empty(),
            &Config::default(),
        );
        let plan = plan(&index, &report);

        assert!(plan.edits.is_empty());
        assert_eq!(plan.skipped.len(), 1);
        assert_eq!(
            plan.skipped[0].reason,
            "removing it would leave the class body empty"
        );
    }
}
