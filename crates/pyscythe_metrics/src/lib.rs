//! Computes [`FunctionMetrics`] for every `def` in a parsed module.
//!
//! Cyclomatic complexity follows `McCabe` as radon counts it: one, plus one for
//! each `if`/`elif`, loop, `except`, `with`, `assert`, ternary, boolean
//! operator, comprehension clause, and `match` case. Cognitive complexity
//! follows the `SonarSource` definition in spirit: each break in linear flow
//! adds one, nested breaks add their nesting depth, `else`/`elif` add one
//! without a nesting bonus, and each boolean-operator sequence adds one.
//! Nested functions are measured on their own and excluded from their parent.

use pyscythe_core::edit::{BodyAfterRemoval, Deletable};
use pyscythe_core::metrics::FunctionMetrics;
use pyscythe_core::source::{ByteOffset, ByteSpan, Column, Line};
use pyscythe_core::symbol::SymbolName;
use pyscythe_core::tokens::{CloneMode, CloneToken};
use ruff_python_ast::token::{TokenKind, Tokens};
use ruff_python_ast::{self as ast, Expr, Stmt};
use ruff_source_file::LineIndex;
use ruff_text_size::Ranged;

/// The significant tokens of a module for clone detection: comments and
/// blank lines are dropped, structure tokens are kept, and identifiers or
/// literals are replaced by placeholders according to `mode`.
#[must_use]
pub fn clone_tokens(tokens: &Tokens, source: &str, mode: CloneMode) -> Vec<CloneToken> {
    let lines = LineIndex::from_source_text(source);
    tokens
        .iter()
        .filter(|token| {
            !matches!(
                token.kind(),
                TokenKind::Comment | TokenKind::NonLogicalNewline | TokenKind::EndOfFile
            )
        })
        .filter_map(|token| {
            let text = match token.kind() {
                TokenKind::Newline => "\\n".to_owned(),
                TokenKind::Indent => "\\t".to_owned(),
                TokenKind::Dedent => "\\d".to_owned(),
                TokenKind::Name if mode != CloneMode::Strict => "$name".to_owned(),
                TokenKind::Int
                | TokenKind::Float
                | TokenKind::Complex
                | TokenKind::String
                | TokenKind::FStringMiddle
                | TokenKind::TStringMiddle
                    if mode == CloneMode::Weak =>
                {
                    "$literal".to_owned()
                }
                _ => source
                    .get(std::ops::Range::<usize>::from(token.range()))?
                    .to_owned(),
            };
            let line = lines.line_column(token.start(), source).line.get();
            Some(CloneToken {
                text,
                line: Line::from_one_based(u32::try_from(line).ok()?)?,
            })
        })
        .collect()
}

/// Definitions at module or class level that can be removed as whole lines.
///
/// `if`/`try` bodies are not descended: a definition guarded by a condition
/// is not safe to delete on the strength of a static reference count.
#[must_use]
pub fn deletables(module: &ast::ModModule, source: &str) -> Vec<Deletable> {
    let lines = LineIndex::from_source_text(source);
    let mut out = Vec::new();
    collect_deletables(&module.body, None, &lines, source, &mut out);
    out
}

fn collect_deletables(
    body: &[Stmt],
    class_body_len: Option<usize>,
    lines: &LineIndex,
    source: &str,
    out: &mut Vec<Deletable>,
) {
    let body_after_removal = match class_body_len {
        Some(len) if len <= 1 => BodyAfterRemoval::WouldBeEmpty,
        _ => BodyAfterRemoval::StillHasStatements,
    };
    for statement in body {
        let (name_identifier, range) = match statement {
            Stmt::FunctionDef(function) => (&function.name, function.range()),
            Stmt::ClassDef(class) => {
                collect_deletables(&class.body, Some(class.body.len()), lines, source, out);
                (&class.name, class.range())
            }
            Stmt::Assign(assign) => match assign.targets.as_slice() {
                [Expr::Name(target)] => {
                    push_deletable(
                        out,
                        target.id.as_str(),
                        target.range(),
                        assign.range(),
                        body_after_removal,
                        lines,
                        source,
                    );
                    continue;
                }
                _ => continue,
            },
            Stmt::AnnAssign(assign) => match &*assign.target {
                Expr::Name(target) => {
                    push_deletable(
                        out,
                        target.id.as_str(),
                        target.range(),
                        assign.range(),
                        body_after_removal,
                        lines,
                        source,
                    );
                    continue;
                }
                _ => continue,
            },
            _ => continue,
        };
        push_deletable(
            out,
            name_identifier.as_str(),
            name_identifier.range(),
            range,
            body_after_removal,
            lines,
            source,
        );
    }
}

fn push_deletable(
    out: &mut Vec<Deletable>,
    name: &str,
    name_range: ruff_text_size::TextRange,
    statement_range: ruff_text_size::TextRange,
    body_after_removal: BodyAfterRemoval,
    lines: &LineIndex,
    source: &str,
) {
    let location = lines.line_column(name_range.start(), source);
    let (Some(line), Some(column)) = (
        u32::try_from(location.line.get())
            .ok()
            .and_then(Line::from_one_based),
        u32::try_from(location.column.get())
            .ok()
            .and_then(Column::from_one_based),
    ) else {
        return;
    };
    let start = lines.line_start(lines.line_index(statement_range.start()), source);
    // Through the end of the last line, newline included; the file end when there is no next line.
    let end_line = lines.line_index(statement_range.end());
    let next_line = end_line.saturating_add(1);
    let end = if next_line.get() <= lines.line_count() {
        lines.line_start(next_line, source)
    } else {
        ruff_text_size::TextSize::of(source)
    };
    out.push(Deletable {
        name: SymbolName::new(name),
        line,
        column,
        lines: ByteSpan::new(
            ByteOffset::new(start.to_u32()),
            ByteOffset::new(end.to_u32()),
        ),
        body_after_removal,
    });
}

/// Metrics for every function and method in `module`, in source order.
#[must_use]
pub fn measure(module: &ast::ModModule, source: &str) -> Vec<FunctionMetrics> {
    let lines = LineIndex::from_source_text(source);
    let mut out = Vec::new();
    collect(&module.body, None, &lines, source, &mut out);
    out
}

fn collect(
    body: &[Stmt],
    owner: Option<&SymbolName>,
    lines: &LineIndex,
    source: &str,
    out: &mut Vec<FunctionMetrics>,
) {
    for statement in body {
        match statement {
            Stmt::FunctionDef(function) => {
                out.push(measure_function(function, owner, lines, source));
                collect(&function.body, None, lines, source, out);
            }
            Stmt::ClassDef(class) => {
                let name = SymbolName::new(class.name.as_str());
                collect(&class.body, Some(&name), lines, source, out);
            }
            Stmt::If(if_statement) => {
                collect(&if_statement.body, owner, lines, source, out);
                for clause in &if_statement.elif_else_clauses {
                    collect(&clause.body, owner, lines, source, out);
                }
            }
            Stmt::Try(try_statement) => {
                collect(&try_statement.body, owner, lines, source, out);
                for handler in &try_statement.handlers {
                    let ast::ExceptHandler::ExceptHandler(handler) = handler;
                    collect(&handler.body, owner, lines, source, out);
                }
                collect(&try_statement.orelse, owner, lines, source, out);
                collect(&try_statement.finalbody, owner, lines, source, out);
            }
            Stmt::With(with_statement) => collect(&with_statement.body, owner, lines, source, out),
            Stmt::For(for_statement) => {
                collect(&for_statement.body, owner, lines, source, out);
                collect(&for_statement.orelse, owner, lines, source, out);
            }
            Stmt::While(while_statement) => {
                collect(&while_statement.body, owner, lines, source, out);
                collect(&while_statement.orelse, owner, lines, source, out);
            }
            _ => {}
        }
    }
}

fn measure_function(
    function: &ast::StmtFunctionDef,
    owner: Option<&SymbolName>,
    lines: &LineIndex,
    source: &str,
) -> FunctionMetrics {
    let mut counter = Counter {
        function_name: function.name.as_str(),
        cyclomatic: 1,
        cognitive: 0,
        max_nesting: 0,
    };
    counter.body(&function.body, 0);

    let first_line = lines.line_column(function.name.start(), source).line.get();
    let last_line = lines.line_column(function.end(), source).line.get();

    FunctionMetrics {
        name: SymbolName::new(function.name.as_str()),
        owner: owner.cloned(),
        name_span: ByteSpan::new(
            ByteOffset::new(function.name.start().to_u32()),
            ByteOffset::new(function.name.end().to_u32()),
        ),
        lines: u32::try_from(last_line.saturating_sub(first_line) + 1).unwrap_or(u32::MAX),
        parameters: u32::try_from(parameter_count(&function.parameters)).unwrap_or(u32::MAX),
        cyclomatic: counter.cyclomatic,
        cognitive: counter.cognitive,
        max_nesting: counter.max_nesting,
    }
}

fn parameter_count(parameters: &ast::Parameters) -> usize {
    parameters.posonlyargs.len()
        + parameters.args.len()
        + parameters.kwonlyargs.len()
        + usize::from(parameters.vararg.is_some())
        + usize::from(parameters.kwarg.is_some())
}

/// Walks one function body, accumulating both complexities.
struct Counter<'a> {
    function_name: &'a str,
    cyclomatic: u32,
    cognitive: u32,
    max_nesting: u32,
}

impl Counter<'_> {
    /// A structural break in linear flow: +1 cyclomatic, +1 cognitive plus the nesting depth.
    fn nested_break(&mut self, nesting: u32) {
        self.cyclomatic += 1;
        self.cognitive += 1 + nesting;
        self.max_nesting = self.max_nesting.max(nesting + 1);
    }

    fn body(&mut self, body: &[Stmt], nesting: u32) {
        for statement in body {
            self.stmt(statement, nesting);
        }
    }

    fn stmt(&mut self, statement: &Stmt, nesting: u32) {
        match statement {
            Stmt::If(if_statement) => {
                self.nested_break(nesting);
                self.expr(&if_statement.test, nesting);
                self.body(&if_statement.body, nesting + 1);
                for clause in &if_statement.elif_else_clauses {
                    // `elif` and `else` add one each but never a nesting bonus.
                    self.cognitive += 1;
                    if let Some(test) = &clause.test {
                        self.cyclomatic += 1;
                        self.expr(test, nesting);
                    }
                    self.body(&clause.body, nesting + 1);
                }
            }
            Stmt::For(for_statement) => {
                self.nested_break(nesting);
                self.expr(&for_statement.iter, nesting);
                self.body(&for_statement.body, nesting + 1);
                if !for_statement.orelse.is_empty() {
                    self.cognitive += 1;
                    self.body(&for_statement.orelse, nesting + 1);
                }
            }
            Stmt::While(while_statement) => {
                self.nested_break(nesting);
                self.expr(&while_statement.test, nesting);
                self.body(&while_statement.body, nesting + 1);
                if !while_statement.orelse.is_empty() {
                    self.cognitive += 1;
                    self.body(&while_statement.orelse, nesting + 1);
                }
            }
            Stmt::Try(try_statement) => {
                self.body(&try_statement.body, nesting);
                for handler in &try_statement.handlers {
                    let ast::ExceptHandler::ExceptHandler(handler) = handler;
                    self.nested_break(nesting);
                    self.body(&handler.body, nesting + 1);
                }
                self.body(&try_statement.orelse, nesting);
                self.body(&try_statement.finalbody, nesting);
            }
            Stmt::With(with_statement) => {
                self.cyclomatic += 1;
                for item in &with_statement.items {
                    self.expr(&item.context_expr, nesting);
                }
                self.body(&with_statement.body, nesting);
            }
            Stmt::Match(match_statement) => {
                self.expr(&match_statement.subject, nesting);
                self.cognitive += 1 + nesting;
                self.max_nesting = self.max_nesting.max(nesting + 1);
                for case in &match_statement.cases {
                    self.cyclomatic += 1;
                    if let Some(guard) = &case.guard {
                        self.expr(guard, nesting + 1);
                    }
                    self.body(&case.body, nesting + 1);
                }
            }
            Stmt::Assert(assert) => {
                self.cyclomatic += 1;
                self.expr(&assert.test, nesting);
            }
            Stmt::Expr(expression) => self.expr(&expression.value, nesting),
            Stmt::Return(ret) => {
                if let Some(value) = &ret.value {
                    self.expr(value, nesting);
                }
            }
            Stmt::Assign(assign) => self.expr(&assign.value, nesting),
            Stmt::AugAssign(assign) => self.expr(&assign.value, nesting),
            Stmt::AnnAssign(assign) => {
                if let Some(value) = &assign.value {
                    self.expr(value, nesting);
                }
            }
            Stmt::Raise(raise) => {
                if let Some(exception) = &raise.exc {
                    self.expr(exception, nesting);
                }
            }
            // Nested definitions are measured separately; everything else is linear flow.
            _ => {}
        }
    }

    fn expr(&mut self, expression: &Expr, nesting: u32) {
        match expression {
            Expr::BoolOp(bool_op) => {
                // Each operator is a decision point; each sequence is one cognitive step.
                self.cyclomatic +=
                    u32::try_from(bool_op.values.len().saturating_sub(1)).unwrap_or(0);
                self.cognitive += 1;
                for value in &bool_op.values {
                    self.expr(value, nesting);
                }
            }
            Expr::If(ternary) => {
                self.nested_break(nesting);
                self.expr(&ternary.test, nesting);
                self.expr(&ternary.body, nesting + 1);
                self.expr(&ternary.orelse, nesting + 1);
            }
            Expr::Lambda(lambda) => {
                self.expr(&lambda.body, nesting + 1);
            }
            Expr::ListComp(comp) => self.comprehension(&comp.elt, &comp.generators, nesting),
            Expr::SetComp(comp) => self.comprehension(&comp.elt, &comp.generators, nesting),
            Expr::Generator(comp) => self.comprehension(&comp.elt, &comp.generators, nesting),
            Expr::DictComp(comp) => {
                if let Some(key) = &comp.key {
                    self.expr(key, nesting + 1);
                }
                self.comprehension(&comp.value, &comp.generators, nesting);
            }
            Expr::Call(call) => {
                if let Expr::Name(callee) = &*call.func
                    && callee.id.as_str() == self.function_name
                {
                    self.cognitive += 1;
                }
                self.expr(&call.func, nesting);
                for argument in &call.arguments.args {
                    self.expr(argument, nesting);
                }
                for keyword in &call.arguments.keywords {
                    self.expr(&keyword.value, nesting);
                }
            }
            Expr::Compare(compare) => {
                self.expr(&compare.left, nesting);
                for comparator in &compare.comparators {
                    self.expr(comparator, nesting);
                }
            }
            Expr::BinOp(binary) => {
                self.expr(&binary.left, nesting);
                self.expr(&binary.right, nesting);
            }
            Expr::UnaryOp(unary) => self.expr(&unary.operand, nesting),
            Expr::Attribute(attribute) => self.expr(&attribute.value, nesting),
            Expr::Subscript(subscript) => {
                self.expr(&subscript.value, nesting);
                self.expr(&subscript.slice, nesting);
            }
            Expr::Tuple(tuple) => {
                for element in &tuple.elts {
                    self.expr(element, nesting);
                }
            }
            Expr::List(list) => {
                for element in &list.elts {
                    self.expr(element, nesting);
                }
            }
            Expr::Set(set) => {
                for element in &set.elts {
                    self.expr(element, nesting);
                }
            }
            Expr::Dict(dict) => {
                for item in &dict.items {
                    if let Some(key) = &item.key {
                        self.expr(key, nesting);
                    }
                    self.expr(&item.value, nesting);
                }
            }
            Expr::Await(awaited) => self.expr(&awaited.value, nesting),
            Expr::Named(named) => self.expr(&named.value, nesting),
            _ => {}
        }
    }

    fn comprehension(&mut self, element: &Expr, generators: &[ast::Comprehension], nesting: u32) {
        for generator in generators {
            self.cyclomatic += 1;
            self.expr(&generator.iter, nesting);
            for condition in &generator.ifs {
                self.cyclomatic += 1;
                self.expr(condition, nesting + 1);
            }
        }
        self.expr(element, nesting + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::{clone_tokens, measure};
    use pyscythe_core::metrics::FunctionMetrics;
    use pyscythe_core::tokens::CloneMode;

    fn texts(source: &str, mode: CloneMode) -> Vec<String> {
        let parsed = ruff_python_parser::parse_module(source).expect("fixture parses");
        clone_tokens(parsed.tokens(), source, mode)
            .into_iter()
            .map(|t| t.text)
            .collect()
    }

    #[test]
    fn strict_tokens_keep_text_and_drop_comments() {
        let tokens = texts("x = 1  # note\n", CloneMode::Strict);
        assert_eq!(tokens, ["x", "=", "1", "\\n"]);
    }

    #[test]
    fn mild_normalises_identifiers_and_weak_also_literals() {
        assert_eq!(
            texts("x = 1\n", CloneMode::Mild),
            ["$name", "=", "1", "\\n"]
        );
        assert_eq!(
            texts("x = 'a'\n", CloneMode::Weak),
            ["$name", "=", "$literal", "\\n"]
        );
    }

    #[test]
    fn tokens_carry_their_line() {
        let parsed = ruff_python_parser::parse_module("a\n\nb\n").expect("parses");
        let tokens = clone_tokens(parsed.tokens(), "a\n\nb\n", CloneMode::Strict);
        let lines: Vec<u32> = tokens.iter().map(|t| t.line.get()).collect();
        assert_eq!(lines, [1, 1, 3, 3]);
    }

    fn metrics(source: &str) -> Vec<FunctionMetrics> {
        let parsed = ruff_python_parser::parse_module(source).expect("fixture parses");
        measure(parsed.syntax(), source)
    }

    fn first(source: &str) -> FunctionMetrics {
        metrics(source).into_iter().next().expect("one function")
    }

    #[test]
    fn deletables_cover_whole_lines_including_decorators_and_flag_sole_methods() {
        let source = "X = 1\n\n\n@dec\ndef f():\n    pass\n\n\nclass C:\n    def only(self):\n        pass\n\n\nclass D:\n    a = 1\n    def m(self):\n        pass\n";
        let parsed = ruff_python_parser::parse_module(source).expect("parses");
        let items = super::deletables(parsed.syntax(), source);
        let names: Vec<&str> = items.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["X", "f", "only", "C", "a", "m", "D"]);

        let f = &items[1];
        assert_eq!((f.line.get(), f.column.get()), (5, 5));
        let block = &source[f.lines.start().get() as usize..f.lines.end().get() as usize];
        assert_eq!(block, "@dec\ndef f():\n    pass\n");

        assert_eq!(
            items[2].body_after_removal,
            pyscythe_core::edit::BodyAfterRemoval::WouldBeEmpty
        );
        assert_eq!(
            items[5].body_after_removal,
            pyscythe_core::edit::BodyAfterRemoval::StillHasStatements
        );
    }

    #[test]
    fn a_straight_line_function_has_complexity_one() {
        let m = first("def f(a, b):\n    return a + b\n");
        assert_eq!((m.cyclomatic, m.cognitive, m.max_nesting), (1, 0, 0));
        assert_eq!(m.parameters, 2);
        assert_eq!(m.lines, 2);
    }

    #[test]
    fn branches_and_loops_count_once_each_for_cyclomatic() {
        let m = first(
            "def f(x):\n    if x:\n        pass\n    elif x > 1:\n        pass\n    else:\n        pass\n    for i in x:\n        pass\n    while x:\n        pass\n    try:\n        pass\n    except ValueError:\n        pass\n    assert x\n    with x:\n        pass\n",
        );
        // 1 + if + elif + for + while + except + assert + with
        assert_eq!(m.cyclomatic, 8);
    }

    #[test]
    fn boolean_operators_ternaries_and_comprehensions_count() {
        let m = first(
            "def f(a, b, c):\n    y = a and b or c\n    z = 1 if a else 2\n    return [i for i in a if i if b]\n",
        );
        // 1 + (and, or: two BoolOp nodes of two values => 2) + ternary + comprehension for + 2 ifs
        assert_eq!(m.cyclomatic, 7);
    }

    #[test]
    fn match_cases_count_as_branches() {
        let m = first(
            "def f(x):\n    match x:\n        case 1:\n            pass\n        case 2 if x:\n            pass\n        case _:\n            pass\n",
        );
        assert_eq!(m.cyclomatic, 4);
        assert_eq!(m.cognitive, 1);
    }

    #[test]
    fn cognitive_complexity_penalises_nesting_but_not_elif_chains() {
        let nested = first(
            "def f(x):\n    if x:\n        for i in x:\n            if i:\n                pass\n",
        );
        // if (+1), for (+1+1), if (+1+2)
        assert_eq!(nested.cognitive, 6);
        assert_eq!(nested.max_nesting, 3);

        let chain = first(
            "def f(x):\n    if x == 1:\n        pass\n    elif x == 2:\n        pass\n    elif x == 3:\n        pass\n    else:\n        pass\n",
        );
        // if (+1), elif (+1), elif (+1), else (+1)
        assert_eq!(chain.cognitive, 4);
        // 1 + if + elif + elif; `else` is not a decision point
        assert_eq!(chain.cyclomatic, 4);
    }

    #[test]
    fn recursion_adds_one_cognitive_point() {
        let m =
            first("def fact(n):\n    if n <= 1:\n        return 1\n    return n * fact(n - 1)\n");
        assert_eq!(m.cognitive, 2);
    }

    #[test]
    fn nested_functions_and_methods_are_measured_separately() {
        let all = metrics(
            "class C:\n    def m(self):\n        def inner(y):\n            if y:\n                pass\n        return inner\n\ndef top():\n    pass\n",
        );
        let names: Vec<_> = all.iter().map(FunctionMetrics::qualified_name).collect();
        assert_eq!(names, ["C.m", "inner", "top"]);
        assert_eq!(all[0].cyclomatic, 1, "inner's branch does not count for m");
        assert_eq!(all[1].cyclomatic, 2);
        assert_eq!(all[0].parameters, 1);
    }
}
