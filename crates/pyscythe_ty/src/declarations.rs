//! Extracts syntax that plugins match on (decorators, base classes, class
//! keywords) from a parsed module, keyed by each definition's name range.

use pyscythe_core::source::ModulePath;
use pyscythe_core::symbol::{Decorator, DottedName, KeywordName};
use ruff_python_ast::name::UnqualifiedName;
use ruff_python_ast::{self as ast, AnyNodeRef, Expr, Stmt};
use ruff_text_size::{Ranged, TextRange};
use rustc_hash::FxHashMap;
use ty_python_semantic::{
    ImportAliasResolution, ResolvedDefinition, SemanticModel, definitions_for_attribute,
    definitions_for_name,
};

/// What a `def` or `class` statement declares beyond its name.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Declaration {
    pub(crate) decorators: Vec<Decorator>,
    pub(crate) bases: Vec<DottedName>,
    pub(crate) class_keywords: Vec<KeywordName>,
}

/// Whether `module` has a top-level `if __name__ == "__main__":` block.
pub(crate) fn has_main_guard(module: &ast::ModModule) -> bool {
    module.body.iter().any(|statement| {
        let Stmt::If(if_statement) = statement else {
            return false;
        };
        let Expr::Compare(compare) = &*if_statement.test else {
            return false;
        };
        let Expr::Name(left) = &*compare.left else {
            return false;
        };
        left.id.as_str() == "__name__"
            && compare
                .comparators
                .iter()
                .any(|c| matches!(c, Expr::StringLiteral(s) if s.value.to_str() == "__main__"))
    })
}

/// Declarations for every `def` and `class` in `module`, keyed by name range.
/// Decorators are resolved through `model` to the module that defines them.
pub(crate) fn declarations_by_name_range(
    module: &ast::ModModule,
    model: &SemanticModel<'_>,
) -> FxHashMap<TextRange, Declaration> {
    let mut out = FxHashMap::default();
    collect_body(&module.body, model, &mut out);
    out
}

fn collect_body(
    body: &[Stmt],
    model: &SemanticModel<'_>,
    out: &mut FxHashMap<TextRange, Declaration>,
) {
    for statement in body {
        match statement {
            Stmt::FunctionDef(function) => {
                out.insert(
                    function.name.range(),
                    Declaration {
                        decorators: decorators_of(&function.decorator_list, model),
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
                        decorators: decorators_of(&class.decorator_list, model),
                        bases,
                        class_keywords,
                    },
                );
                collect_body(&class.body, model, out);
            }
            Stmt::If(if_statement) => {
                collect_body(&if_statement.body, model, out);
                for clause in &if_statement.elif_else_clauses {
                    collect_body(&clause.body, model, out);
                }
            }
            Stmt::Try(try_statement) => {
                collect_body(&try_statement.body, model, out);
                for handler in &try_statement.handlers {
                    let ast::ExceptHandler::ExceptHandler(handler) = handler;
                    collect_body(&handler.body, model, out);
                }
                collect_body(&try_statement.orelse, model, out);
                collect_body(&try_statement.finalbody, model, out);
            }
            Stmt::With(with_statement) => collect_body(&with_statement.body, model, out),
            _ => {}
        }
    }
}

fn decorators_of(list: &[ast::Decorator], model: &SemanticModel<'_>) -> Vec<Decorator> {
    list.iter()
        .filter_map(|decorator| decorator_of(decorator, model))
        .collect()
}

/// The module defining what `callee` refers to, when ty can resolve it.
fn defining_module(callee: &Expr, model: &SemanticModel<'_>) -> Option<ModulePath> {
    let resolved = match callee {
        Expr::Name(name) => definitions_for_name(
            model,
            name.id.as_str(),
            AnyNodeRef::from(name),
            ImportAliasResolution::ResolveAliases,
        ),
        Expr::Attribute(attribute) => definitions_for_attribute(model, attribute),
        _ => return None,
    };
    let db = model.db();
    resolved.iter().find_map(|definition| {
        let file = match definition {
            ResolvedDefinition::Definition(def) => {
                // When ty cannot follow an import to its origin it answers with
                // the local import binding; that says nothing about where the
                // decorator really comes from.
                if def.file(db) == model.file() && def.kind(db).is_import() {
                    return None;
                }
                def.file(db)
            }
            ResolvedDefinition::Module(module) => module.file(db),
            ResolvedDefinition::FileWithRange(range) => range.file(),
        };
        crate::module_name_of(db, file).map(ModulePath::new)
    })
}

fn decorator_of(decorator: &ast::Decorator, model: &SemanticModel<'_>) -> Option<Decorator> {
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
        module: defining_module(callee, model),
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
