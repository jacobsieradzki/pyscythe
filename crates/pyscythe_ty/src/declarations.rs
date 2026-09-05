//! Extracts syntax that plugins match on (decorators, base classes, class
//! keywords) from a parsed module, keyed by each definition's name range.

use pyscythe_core::symbol::{Decorator, DottedName, KeywordName};
use ruff_python_ast::name::UnqualifiedName;
use ruff_python_ast::{self as ast, Expr, Stmt};
use ruff_text_size::{Ranged, TextRange};
use rustc_hash::FxHashMap;

/// What a `def` or `class` statement declares beyond its name.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Declaration {
    pub(crate) decorators: Vec<Decorator>,
    pub(crate) bases: Vec<DottedName>,
    pub(crate) class_keywords: Vec<KeywordName>,
}

/// Declarations for every `def` and `class` in `module`, keyed by name range.
pub(crate) fn declarations_by_name_range(
    module: &ast::ModModule,
) -> FxHashMap<TextRange, Declaration> {
    let mut out = FxHashMap::default();
    collect_body(&module.body, &mut out);
    out
}

fn collect_body(body: &[Stmt], out: &mut FxHashMap<TextRange, Declaration>) {
    for statement in body {
        match statement {
            Stmt::FunctionDef(function) => {
                out.insert(
                    function.name.range(),
                    Declaration {
                        decorators: decorators_of(&function.decorator_list),
                        ..Declaration::default()
                    },
                );
            }
            Stmt::ClassDef(class) => {
                let (bases, class_keywords) = class.arguments.as_ref().map_or_else(
                    || (Vec::new(), Vec::new()),
                    |arguments| {
                        (
                            arguments.args.iter().filter_map(dotted_name_of).collect(),
                            arguments
                                .keywords
                                .iter()
                                .filter_map(|keyword| keyword.arg.as_ref())
                                .map(|arg| KeywordName::new(arg.as_str()))
                                .collect(),
                        )
                    },
                );
                out.insert(
                    class.name.range(),
                    Declaration {
                        decorators: decorators_of(&class.decorator_list),
                        bases,
                        class_keywords,
                    },
                );
                collect_body(&class.body, out);
            }
            Stmt::If(if_statement) => {
                collect_body(&if_statement.body, out);
                for clause in &if_statement.elif_else_clauses {
                    collect_body(&clause.body, out);
                }
            }
            Stmt::Try(try_statement) => {
                collect_body(&try_statement.body, out);
                for handler in &try_statement.handlers {
                    let ast::ExceptHandler::ExceptHandler(handler) = handler;
                    collect_body(&handler.body, out);
                }
                collect_body(&try_statement.orelse, out);
                collect_body(&try_statement.finalbody, out);
            }
            Stmt::With(with_statement) => collect_body(&with_statement.body, out),
            _ => {}
        }
    }
}

fn decorators_of(list: &[ast::Decorator]) -> Vec<Decorator> {
    list.iter().filter_map(decorator_of).collect()
}

fn decorator_of(decorator: &ast::Decorator) -> Option<Decorator> {
    let (callee, keywords) = match &decorator.expression {
        Expr::Call(call) => (
            &*call.func,
            call.arguments
                .keywords
                .iter()
                .filter_map(|keyword| keyword.arg.as_ref())
                .map(|arg| KeywordName::new(arg.as_str()))
                .collect(),
        ),
        other => (other, Vec::new()),
    };
    Some(Decorator {
        name: dotted_name_of(callee)?,
        keywords,
    })
}

/// The dotted name of a `Name`/`Attribute` chain; subscripts such as `Generic[T]` are stripped.
fn dotted_name_of(expr: &Expr) -> Option<DottedName> {
    let target = match expr {
        Expr::Subscript(subscript) => &*subscript.value,
        other => other,
    };
    UnqualifiedName::from_expr(target).map(|name| DottedName::new(name.to_string()))
}
