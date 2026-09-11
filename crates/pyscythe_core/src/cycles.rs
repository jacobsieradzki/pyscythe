//! Finds groups of modules that import each other at load time.
//!
//! Deferred imports (inside functions) and `TYPE_CHECKING` imports do not run
//! when a module loads, so they cannot cause an import-time cycle and are
//! ignored here.

use std::collections::BTreeMap;

use crate::finding::{Confidence, Detail, Finding, Location, Rule};
use crate::index::{CodebaseIndex, Import, ImportKind};
use crate::report::{Report, ReportKind, Summary};
use crate::source::FileId;

/// Which imports take part in the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CycleOptions {
    /// Also follow imports inside function bodies, which only bite when called.
    pub include_deferred: bool,
}

/// Stop enumerating after this many cycles in one strongly connected component.
const MAX_CYCLES_PER_COMPONENT: usize = 25;

/// Runs the circular-import analysis over every file in `index`.
#[must_use]
pub fn analyze(index: &dyn CodebaseIndex, options: CycleOptions) -> Report {
    let files = index.files();
    let graph = Graph::imports(index, options);

    let mut findings: Vec<Finding> = graph
        .strongly_connected_components()
        .into_iter()
        .filter(|component| component.len() > 1)
        .flat_map(|component| graph.simple_cycles(&component))
        .filter_map(|cycle| finding_for(index, &cycle))
        .collect();
    findings.sort_by(|a, b| a.path.cmp(&b.path).then(a.position.cmp(&b.position)));
    findings.dedup();

    Report {
        schema_version: Report::SCHEMA_VERSION,
        kind: ReportKind::Cycles,
        kept: Vec::new(),
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
    }
}

/// Runtime import edges between files known to the index.
struct Graph {
    edges: BTreeMap<FileId, Vec<Import>>,
}

impl Graph {
    fn imports(index: &dyn CodebaseIndex, options: CycleOptions) -> Self {
        let counts = |kind: ImportKind| match kind {
            ImportKind::Runtime => true,
            ImportKind::Deferred => options.include_deferred,
            ImportKind::TypeOnly => false,
        };
        let edges = index
            .files()
            .iter()
            .map(|file| {
                let imports = index
                    .imports(file.id)
                    .into_iter()
                    .filter(|import| counts(import.kind) && import.target != file.id)
                    .collect();
                (file.id, imports)
            })
            .collect();
        Self { edges }
    }

    fn successors(&self, file: FileId) -> impl Iterator<Item = &Import> {
        self.edges.get(&file).into_iter().flatten()
    }

    /// Tarjan's algorithm, iterative so deep import chains cannot overflow the stack.
    fn strongly_connected_components(&self) -> Vec<Vec<FileId>> {
        let mut state = Tarjan::default();
        for &file in self.edges.keys() {
            if !state.index_of.contains_key(&file) {
                state.visit(self, file);
            }
        }
        state.components
    }

    /// Every simple cycle inside `component`, each as the edges that form it,
    /// capped at [`MAX_CYCLES_PER_COMPONENT`]. Cycles through the smallest
    /// node are enumerated first, then that node is removed and the rest
    /// recursed, so no rotation of the same cycle appears twice.
    fn simple_cycles(&self, component: &[FileId]) -> Vec<Vec<(FileId, Import)>> {
        let mut remaining: Vec<FileId> = component.to_vec();
        remaining.sort_unstable();
        let mut cycles = Vec::new();
        while let Some(&start) = remaining.first() {
            let mut path: Vec<(FileId, Import)> = Vec::new();
            let mut on_path: Vec<FileId> = vec![start];
            self.cycles_from(
                start,
                start,
                &remaining,
                &mut path,
                &mut on_path,
                &mut cycles,
            );
            if cycles.len() >= MAX_CYCLES_PER_COMPONENT {
                break;
            }
            remaining.remove(0);
        }
        cycles.truncate(MAX_CYCLES_PER_COMPONENT);
        cycles
    }

    fn cycles_from(
        &self,
        start: FileId,
        current: FileId,
        allowed: &[FileId],
        path: &mut Vec<(FileId, Import)>,
        on_path: &mut Vec<FileId>,
        out: &mut Vec<Vec<(FileId, Import)>>,
    ) {
        for import in self.successors(current) {
            if out.len() >= MAX_CYCLES_PER_COMPONENT {
                return;
            }
            if import.target == start {
                let mut cycle = path.clone();
                cycle.push((current, *import));
                out.push(cycle);
                continue;
            }
            if !allowed.contains(&import.target) || on_path.contains(&import.target) {
                continue;
            }
            path.push((current, *import));
            on_path.push(import.target);
            self.cycles_from(start, import.target, allowed, path, on_path, out);
            on_path.pop();
            path.pop();
        }
    }
}

#[derive(Default)]
struct Tarjan {
    counter: usize,
    index_of: BTreeMap<FileId, usize>,
    low_link: BTreeMap<FileId, usize>,
    stack: Vec<FileId>,
    on_stack: Vec<FileId>,
    components: Vec<Vec<FileId>>,
}

impl Tarjan {
    fn visit(&mut self, graph: &Graph, root: FileId) {
        // Each frame is (node, successors still to explore).
        let mut frames: Vec<(FileId, Vec<FileId>)> = Vec::new();
        self.enter(root);
        frames.push((root, graph.successors(root).map(|i| i.target).collect()));

        while let Some((node, pending)) = frames.last_mut() {
            let node = *node;
            if let Some(next) = pending.pop() {
                if !self.index_of.contains_key(&next) {
                    self.enter(next);
                    frames.push((next, graph.successors(next).map(|i| i.target).collect()));
                } else if self.on_stack.contains(&next) {
                    let candidate = self.index_of.get(&next).copied().unwrap_or(usize::MAX);
                    self.lower(node, candidate);
                }
                continue;
            }

            frames.pop();
            if let Some((parent, _)) = frames.last() {
                let child_low = self.low_link.get(&node).copied().unwrap_or(usize::MAX);
                self.lower(*parent, child_low);
            }
            if self.low_link.get(&node) == self.index_of.get(&node) {
                let mut component = Vec::new();
                while let Some(member) = self.stack.pop() {
                    self.on_stack.retain(|f| *f != member);
                    component.push(member);
                    if member == node {
                        break;
                    }
                }
                component.sort_unstable();
                self.components.push(component);
            }
        }
    }

    fn enter(&mut self, node: FileId) {
        self.index_of.insert(node, self.counter);
        self.low_link.insert(node, self.counter);
        self.counter += 1;
        self.stack.push(node);
        self.on_stack.push(node);
    }

    fn lower(&mut self, node: FileId, candidate: usize) {
        if let Some(low) = self.low_link.get_mut(&node)
            && candidate < *low
        {
            *low = candidate;
        }
    }
}

fn finding_for(index: &dyn CodebaseIndex, cycle: &[(FileId, Import)]) -> Option<Finding> {
    let (first_file, first_import) = cycle.first().copied()?;

    let chain: Vec<Location> = cycle
        .iter()
        .filter_map(|(from, import)| {
            let file = index.file(*from)?;
            Some(Location {
                path: file.path.clone(),
                module: file.module.clone(),
                position: index.position(*from, import.span.start()),
            })
        })
        .collect();

    let names: Vec<String> = chain
        .iter()
        .map(|link| {
            link.module
                .as_ref()
                .map_or_else(|| link.path.to_string(), |m| m.as_str().to_owned())
        })
        .chain(
            chain
                .first()
                .and_then(|first| first.module.as_ref())
                .map(|m| m.as_str().to_owned()),
        )
        .collect();

    let file = index.file(first_file)?;
    Some(Finding {
        rule: Rule::CircularImport,
        path: file.path.clone(),
        module: file.module.clone(),
        position: index.position(first_file, first_import.span.start()),
        confidence: Confidence::High,
        message: format!("import cycle: {}", names.join(" -> ")),
        detail: Detail::Cycle { chain },
    })
}

#[cfg(test)]
mod tests {
    use super::{CycleOptions, analyze};
    use crate::finding::Detail;
    use crate::index::ImportKind;
    use crate::testing::FakeIndex;

    #[test]
    fn two_modules_importing_each_other_form_one_cycle() {
        let mut index = FakeIndex::new();
        let a = index.add_file("/proj/pkg/a.py", "pkg.a");
        let b = index.add_file("/proj/pkg/b.py", "pkg.b");
        index.add_import(a, b, ImportKind::Runtime);
        index.add_import(b, a, ImportKind::Runtime);

        let report = analyze(&index, CycleOptions::default());

        assert_eq!(report.findings.len(), 1);
        assert_eq!(
            report.findings[0].message,
            "import cycle: pkg.a -> pkg.b -> pkg.a"
        );
        let Detail::Cycle { chain } = &report.findings[0].detail else {
            panic!("expected a cycle detail");
        };
        assert_eq!(chain.len(), 2);
    }

    #[test]
    fn deferred_and_type_only_imports_do_not_close_cycles() {
        let mut index = FakeIndex::new();
        let a = index.add_file("/proj/pkg/a.py", "pkg.a");
        let b = index.add_file("/proj/pkg/b.py", "pkg.b");
        let c = index.add_file("/proj/pkg/c.py", "pkg.c");
        index.add_import(a, b, ImportKind::Runtime);
        index.add_import(b, a, ImportKind::TypeOnly);
        index.add_import(a, c, ImportKind::Runtime);
        index.add_import(c, a, ImportKind::Deferred);

        assert!(analyze(&index, CycleOptions::default()).is_clean());
    }

    #[test]
    fn a_longer_cycle_is_reported_once_from_its_first_module() {
        let mut index = FakeIndex::new();
        let a = index.add_file("/proj/pkg/a.py", "pkg.a");
        let b = index.add_file("/proj/pkg/b.py", "pkg.b");
        let c = index.add_file("/proj/pkg/c.py", "pkg.c");
        index.add_import(a, b, ImportKind::Runtime);
        index.add_import(b, c, ImportKind::Runtime);
        index.add_import(c, a, ImportKind::Runtime);

        let report = analyze(&index, CycleOptions::default());

        assert_eq!(report.findings.len(), 1);
        assert_eq!(
            report.findings[0].module.as_ref().unwrap().as_str(),
            "pkg.a"
        );
        assert_eq!(
            report.findings[0].message,
            "import cycle: pkg.a -> pkg.b -> pkg.c -> pkg.a"
        );
    }

    #[test]
    fn every_distinct_cycle_in_a_component_is_reported() {
        let mut index = FakeIndex::new();
        let a = index.add_file("/proj/pkg/a.py", "pkg.a");
        let b = index.add_file("/proj/pkg/b.py", "pkg.b");
        let c = index.add_file("/proj/pkg/c.py", "pkg.c");
        index.add_import(a, b, ImportKind::Runtime);
        index.add_import(b, a, ImportKind::Runtime);
        index.add_import(b, c, ImportKind::Runtime);
        index.add_import(c, b, ImportKind::Runtime);
        index.add_import(c, a, ImportKind::Runtime);

        let report = analyze(&index, CycleOptions::default());

        let mut messages: Vec<&str> = report.findings.iter().map(|f| f.message.as_str()).collect();
        messages.sort_unstable();
        assert_eq!(
            messages,
            [
                "import cycle: pkg.a -> pkg.b -> pkg.a",
                "import cycle: pkg.a -> pkg.b -> pkg.c -> pkg.a",
                "import cycle: pkg.b -> pkg.c -> pkg.b",
            ]
        );
    }

    #[test]
    fn deferred_imports_count_when_asked() {
        let mut index = FakeIndex::new();
        let a = index.add_file("/proj/pkg/a.py", "pkg.a");
        let b = index.add_file("/proj/pkg/b.py", "pkg.b");
        index.add_import(a, b, ImportKind::Runtime);
        index.add_import(b, a, ImportKind::Deferred);

        assert!(analyze(&index, CycleOptions::default()).is_clean());
        let with_deferred = analyze(
            &index,
            CycleOptions {
                include_deferred: true,
            },
        );
        assert_eq!(with_deferred.findings.len(), 1);
    }

    #[test]
    fn a_chain_without_a_loop_is_clean() {
        let mut index = FakeIndex::new();
        let a = index.add_file("/proj/pkg/a.py", "pkg.a");
        let b = index.add_file("/proj/pkg/b.py", "pkg.b");
        index.add_import(a, b, ImportKind::Runtime);

        assert!(analyze(&index, CycleOptions::default()).is_clean());
    }
}
