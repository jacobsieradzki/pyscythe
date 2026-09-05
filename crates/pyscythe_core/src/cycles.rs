//! Finds groups of modules that import each other at load time.
//!
//! Deferred imports (inside functions) and `TYPE_CHECKING` imports do not run
//! when a module loads, so they cannot cause an import-time cycle and are
//! ignored here.

use std::collections::BTreeMap;

use crate::finding::{Confidence, Detail, Finding, Location, Rule};
use crate::index::{CodebaseIndex, Import, ImportKind};
use crate::report::{Report, ReportKind, Summary};
use crate::source::{FileId, SourceFile};

/// Runs the circular-import analysis over every file in `index`.
#[must_use]
pub fn analyze(index: &dyn CodebaseIndex) -> Report {
    let files = index.files();
    let graph = Graph::runtime_imports(index);

    let mut findings: Vec<Finding> = graph
        .strongly_connected_components()
        .into_iter()
        .filter(|component| component.len() > 1)
        .filter_map(|component| finding_for(index, files, &graph, &component))
        .collect();
    findings.sort_by(|a, b| a.path.cmp(&b.path).then(a.position.cmp(&b.position)));

    Report {
        schema_version: Report::SCHEMA_VERSION,
        kind: ReportKind::Cycles,
        summary: Summary {
            files_scanned: files.len(),
            symbols_checked: 0,
            symbols_kept: 0,
            symbols_ignored: 0,
            findings: findings.len(),
        },
        findings,
    }
}

/// Runtime import edges between files known to the index.
struct Graph {
    edges: BTreeMap<FileId, Vec<Import>>,
}

impl Graph {
    fn runtime_imports(index: &dyn CodebaseIndex) -> Self {
        let edges = index
            .files()
            .iter()
            .map(|file| {
                let imports = index
                    .imports(file.id)
                    .into_iter()
                    .filter(|import| import.kind == ImportKind::Runtime && import.target != file.id)
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

    /// A simple cycle through `component` starting at `start`, as the edges that close it.
    fn cycle_from(&self, start: FileId, component: &[FileId]) -> Vec<(FileId, Import)> {
        let in_component = |file: FileId| component.contains(&file);
        let mut path: Vec<(FileId, Import)> = Vec::new();
        let mut visited = vec![start];
        let mut current = start;
        loop {
            let Some(next) = self.successors(current).find(|import| {
                in_component(import.target)
                    && (import.target == start || !visited.contains(&import.target))
            }) else {
                // Dead end inside the component; back up one step.
                match path.pop() {
                    Some((previous, _)) => {
                        current = previous;
                        continue;
                    }
                    None => return Vec::new(),
                }
            };
            path.push((current, *next));
            if next.target == start {
                return path;
            }
            visited.push(next.target);
            current = next.target;
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

fn finding_for(
    index: &dyn CodebaseIndex,
    files: &[SourceFile],
    graph: &Graph,
    component: &[FileId],
) -> Option<Finding> {
    let start = *component
        .iter()
        .min_by_key(|id| files.get(id.index()).map(|f| &f.path))?;
    let cycle = graph.cycle_from(start, component);
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
    use super::analyze;
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

        let report = analyze(&index);

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

        assert!(analyze(&index).is_clean());
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

        let report = analyze(&index);

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
    fn a_chain_without_a_loop_is_clean() {
        let mut index = FakeIndex::new();
        let a = index.add_file("/proj/pkg/a.py", "pkg.a");
        let b = index.add_file("/proj/pkg/b.py", "pkg.b");
        index.add_import(a, b, ImportKind::Runtime);

        assert!(analyze(&index).is_clean());
    }
}
