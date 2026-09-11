//! Finds runs of tokens that appear in more than one place.
//!
//! Every window of `min_tokens` tokens is hashed; windows that collide are
//! verified and extended as far as they stay equal. Longer clones win when
//! they overlap shorter ones, which keeps the report to maximal matches.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::finding::{Confidence, Detail, Finding, Location, Rule};
use crate::index::CodebaseIndex;
use crate::report::{DuplicationSummary, Report, ReportKind, Summary};
use crate::source::{FileId, Line};
use crate::tokens::{CloneMode, CloneToken};

/// What counts as a clone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DupesOptions {
    /// How tokens are normalised before comparison.
    pub mode: CloneMode,
    /// Shortest run of tokens that can be a clone.
    pub min_tokens: usize,
    /// Shortest run of lines that can be a clone.
    pub min_lines: u32,
}

impl Default for DupesOptions {
    fn default() -> Self {
        Self {
            mode: CloneMode::Mild,
            min_tokens: 50,
            min_lines: 5,
        }
    }
}

/// Runs clone detection over every file in `index`.
#[must_use]
pub fn analyze(index: &dyn CodebaseIndex, options: &DupesOptions) -> Report {
    let files = index.files();
    let streams: Vec<TokenStream> = files
        .iter()
        .map(|file| TokenStream::intern(file.id, index.clone_tokens(file.id, options.mode)))
        .collect();
    let mut interner = Interner::default();
    let streams: Vec<TokenStream> = streams
        .into_iter()
        .map(|stream| stream.with_ids(&mut interner))
        .collect();

    let clones = maximal_clones(&streams, options);

    let mut findings: Vec<Finding> = clones
        .iter()
        .filter_map(|clone| finding_for(index, &streams, clone))
        .collect();
    findings.sort_by(|a, b| a.path.cmp(&b.path).then(a.position.cmp(&b.position)));

    let duplicated_lines = duplicated_lines(&streams, &clones);
    let total_lines: u32 = streams.iter().map(TokenStream::line_count).sum();
    let percent_tenths = if total_lines == 0 {
        0
    } else {
        u32::try_from(u64::from(duplicated_lines) * 1000 / u64::from(total_lines))
            .unwrap_or(u32::MAX)
    };

    Report {
        schema_version: Report::SCHEMA_VERSION,
        kind: ReportKind::Dupes,
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
            duplication: Some(DuplicationSummary {
                clones: clones.len(),
                duplicated_lines,
                total_lines,
                percent_tenths,
            }),
        },
        findings,
        kept: Vec::new(),
    }
}

/// Assigns each distinct token text a small integer so windows compare fast.
#[derive(Default)]
struct Interner {
    ids: HashMap<String, u32>,
}

impl Interner {
    fn id(&mut self, text: &str) -> u32 {
        if let Some(&id) = self.ids.get(text) {
            return id;
        }
        let id = u32::try_from(self.ids.len()).unwrap_or(u32::MAX);
        self.ids.insert(text.to_owned(), id);
        id
    }
}

/// One file's tokens as ids, with the line of each.
struct TokenStream {
    file: FileId,
    texts: Vec<String>,
    ids: Vec<u32>,
    lines: Vec<Line>,
}

impl TokenStream {
    fn intern(file: FileId, tokens: Vec<CloneToken>) -> Self {
        let (texts, lines) = tokens.into_iter().map(|t| (t.text, t.line)).unzip();
        Self {
            file,
            texts,
            ids: Vec::new(),
            lines,
        }
    }

    fn with_ids(mut self, interner: &mut Interner) -> Self {
        self.ids = self.texts.iter().map(|text| interner.id(text)).collect();
        self.texts = Vec::new();
        self
    }

    fn line_at(&self, index: usize) -> Option<Line> {
        self.lines.get(index).copied()
    }

    fn line_count(&self) -> u32 {
        self.lines.last().map_or(0, |line| line.get())
    }
}

/// A clone: `length` tokens starting at `a` and again at `b`, with `a` first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Clone {
    a: Position,
    b: Position,
    length: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Position {
    stream: usize,
    token: usize,
}

fn maximal_clones(streams: &[TokenStream], options: &DupesOptions) -> Vec<Clone> {
    let window = options.min_tokens.max(1);
    let mut by_window: BTreeMap<Vec<u32>, Vec<Position>> = BTreeMap::new();
    for (stream_index, stream) in streams.iter().enumerate() {
        for (token, ids) in stream.ids.windows(window).enumerate() {
            by_window.entry(ids.to_vec()).or_default().push(Position {
                stream: stream_index,
                token,
            });
        }
    }

    let mut candidates: Vec<Clone> = Vec::new();
    for positions in by_window.values().filter(|p| p.len() > 1) {
        for (i, &a) in positions.iter().enumerate() {
            for &b in positions.iter().skip(i + 1) {
                let length = common_prefix(streams, a, b);
                if a.stream == b.stream && b.token < a.token + length {
                    // Overlapping runs inside one file are self-similarity, not a clone.
                    continue;
                }
                candidates.push(Clone { a, b, length });
            }
        }
    }

    candidates.sort_by(|x, y| {
        y.length
            .cmp(&x.length)
            .then(x.a.cmp(&y.a))
            .then(x.b.cmp(&y.b))
    });

    let mut claimed: Vec<Vec<bool>> = streams.iter().map(|s| vec![false; s.ids.len()]).collect();
    let mut kept = Vec::new();
    for clone in candidates {
        if !meets_line_minimum(streams, clone, options.min_lines) {
            continue;
        }
        let taken = |position: Position| {
            claimed
                .get(position.stream)
                .and_then(|flags| flags.get(position.token..position.token + clone.length))
                .is_some_and(|window| window.iter().any(|&f| f))
        };
        if taken(clone.a) || taken(clone.b) {
            continue;
        }
        for position in [clone.a, clone.b] {
            if let Some(window) = claimed
                .get_mut(position.stream)
                .and_then(|flags| flags.get_mut(position.token..position.token + clone.length))
            {
                window.fill(true);
            }
        }
        kept.push(clone);
    }
    kept
}

fn common_prefix(streams: &[TokenStream], a: Position, b: Position) -> usize {
    let (Some(sa), Some(sb)) = (streams.get(a.stream), streams.get(b.stream)) else {
        return 0;
    };
    let (Some(rest_a), Some(rest_b)) = (sa.ids.get(a.token..), sb.ids.get(b.token..)) else {
        return 0;
    };
    rest_a
        .iter()
        .zip(rest_b)
        .take_while(|(x, y)| x == y)
        .count()
}

fn line_span(streams: &[TokenStream], position: Position, length: usize) -> Option<(Line, Line)> {
    let stream = streams.get(position.stream)?;
    let start = stream.line_at(position.token)?;
    let end = stream.line_at(position.token + length - 1)?;
    Some((start, end))
}

fn meets_line_minimum(streams: &[TokenStream], clone: Clone, min_lines: u32) -> bool {
    line_span(streams, clone.a, clone.length)
        .is_some_and(|(start, end)| end.get() - start.get() + 1 >= min_lines)
}

fn duplicated_lines(streams: &[TokenStream], clones: &[Clone]) -> u32 {
    let mut covered: Vec<BTreeSet<u32>> = streams.iter().map(|_| BTreeSet::default()).collect();
    for clone in clones {
        for position in [clone.a, clone.b] {
            if let (Some((start, end)), Some(set)) = (
                line_span(streams, position, clone.length),
                covered.get_mut(position.stream),
            ) {
                set.extend(start.get()..=end.get());
            }
        }
    }
    covered
        .iter()
        .map(|set| u32::try_from(set.len()).unwrap_or(u32::MAX))
        .sum()
}

fn finding_for(
    index: &dyn CodebaseIndex,
    streams: &[TokenStream],
    clone: &Clone,
) -> Option<Finding> {
    let (a_start, a_end) = line_span(streams, clone.a, clone.length)?;
    let (b_start, b_end) = line_span(streams, clone.b, clone.length)?;
    let file_a = index.file(streams.get(clone.a.stream)?.file)?;
    let file_b = index.file(streams.get(clone.b.stream)?.file)?;
    let column = crate::source::Column::from_one_based(1)?;
    let other = Location {
        path: file_b.path.clone(),
        module: file_b.module.clone(),
        position: Some(crate::source::Position {
            line: b_start,
            column,
        }),
    };
    let lines = a_end.get() - a_start.get() + 1;
    let other_display = file_b
        .module
        .as_ref()
        .map_or_else(|| file_b.path.to_string(), |m| m.as_str().to_owned());
    Some(Finding {
        rule: Rule::DuplicateCode,
        path: file_a.path.clone(),
        module: file_a.module.clone(),
        position: Some(crate::source::Position {
            line: a_start,
            column,
        }),
        confidence: Confidence::High,
        message: format!(
            "{lines} lines ({} tokens) duplicated at {other_display}:{}",
            clone.length,
            b_start.get()
        ),
        detail: Detail::Duplicate {
            lines,
            tokens: u32::try_from(clone.length).unwrap_or(u32::MAX),
            end_line: a_end.get(),
            other,
            other_end_line: b_end.get(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::{DupesOptions, analyze};
    use crate::finding::Detail;
    use crate::testing::FakeIndex;
    use crate::tokens::CloneMode;

    fn options(min_tokens: usize, min_lines: u32) -> DupesOptions {
        DupesOptions {
            mode: CloneMode::Mild,
            min_tokens,
            min_lines,
        }
    }

    const BLOCK: &str = "a b c\nd e f\ng h i\nj k l\nm n o\n";

    #[test]
    fn an_identical_block_in_two_files_is_one_clone() {
        let mut index = FakeIndex::new();
        let a = index.add_file("/proj/pkg/a.py", "pkg.a");
        let b = index.add_file("/proj/pkg/b.py", "pkg.b");
        index.set_tokens(a, &format!("x y z\n{BLOCK}"));
        index.set_tokens(b, &format!("{BLOCK}q r s\n"));

        let report = analyze(&index, &options(10, 3));

        assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
        let finding = &report.findings[0];
        assert_eq!(finding.position.unwrap().line.get(), 2);
        let Detail::Duplicate {
            lines,
            tokens,
            other,
            ..
        } = &finding.detail
        else {
            panic!("expected a duplicate detail");
        };
        assert_eq!((*lines, *tokens), (5, 15));
        assert_eq!(other.position.unwrap().line.get(), 1);
        let duplication = report.summary.duplication.unwrap();
        assert_eq!(duplication.clones, 1);
        assert_eq!(duplication.duplicated_lines, 10);
    }

    #[test]
    fn short_matches_are_ignored() {
        let mut index = FakeIndex::new();
        let a = index.add_file("/proj/pkg/a.py", "pkg.a");
        let b = index.add_file("/proj/pkg/b.py", "pkg.b");
        index.set_tokens(a, BLOCK);
        index.set_tokens(b, BLOCK);

        assert!(
            analyze(&index, &options(50, 3)).is_clean(),
            "too few tokens"
        );
        assert!(
            analyze(&index, &options(10, 20)).is_clean(),
            "too few lines"
        );
    }

    #[test]
    fn a_clone_within_one_file_is_reported_once_without_overlap() {
        let mut index = FakeIndex::new();
        let a = index.add_file("/proj/pkg/a.py", "pkg.a");
        index.set_tokens(a, &format!("{BLOCK}---\n{BLOCK}"));

        let report = analyze(&index, &options(10, 3));

        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].position.unwrap().line.get(), 1);
    }

    #[test]
    fn overlapping_candidates_collapse_to_the_longest_match() {
        let mut index = FakeIndex::new();
        let a = index.add_file("/proj/pkg/a.py", "pkg.a");
        let b = index.add_file("/proj/pkg/b.py", "pkg.b");
        let long = format!("{BLOCK}p q r\ns t u\n");
        index.set_tokens(a, &long);
        index.set_tokens(b, &long);

        let report = analyze(&index, &options(6, 3));

        assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
        let Detail::Duplicate { tokens, .. } = report.findings[0].detail else {
            panic!("expected a duplicate detail");
        };
        assert_eq!(tokens, 21);
    }
}
