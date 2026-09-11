//! Extracts syntax that plugins match on (decorators, base classes, class
//! keywords) from a parsed module, keyed by each definition's name range.

use pyscythe_core::source::ModulePath;
use pyscythe_core::symbol::{Decorator, DottedName, KeywordName, Provenance};
use ruff_python_ast::name::UnqualifiedName;
use ruff_python_ast::{self as ast, AnyNodeRef, Expr, Stmt};
use ruff_text_size::{Ranged, TextRange};
use rustc_hash::{FxHashMap, FxHashSet};
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

/// The names listed in `__all__` at module level: `__all__ = [...]`,
/// `__all__ += [...]`, and `__all__.extend([...])`, string literals only.
pub(crate) fn dunder_all_names(module: &ast::ModModule) -> Vec<String> {
    let mut names = Vec::new();
    let mut push_all = |value: &Expr| {
        let elements = match value {
            Expr::List(list) => &list.elts,
            Expr::Tuple(tuple) => &tuple.elts,
            _ => return,
        };
        for element in elements {
            if let Expr::StringLiteral(literal) = element {
                names.push(literal.value.to_str().to_owned());
            }
        }
    };
    for statement in &module.body {
        match statement {
            Stmt::Assign(assign) if assign.targets.iter().any(is_dunder_all) => {
                push_all(&assign.value);
            }
            Stmt::AugAssign(assign) if is_dunder_all(&assign.target) => push_all(&assign.value),
            Stmt::Expr(expression) => {
                if let Expr::Call(call) = &*expression.value
                    && let Expr::Attribute(attribute) = &*call.func
                    && attribute.attr.as_str() == "extend"
                    && is_dunder_all(&attribute.value)
                    && let Some(argument) = call.arguments.args.first()
                {
                    push_all(argument);
                }
            }
            _ => {}
        }
    }
    names
}

fn is_dunder_all(expr: &Expr) -> bool {
    matches!(expr, Expr::Name(name) if name.id.as_str() == "__all__")
}

/// Name ranges bound by plain, annotated, or augmented assignment statements
/// at module level and in class bodies, including inside `if`/`try`/`with`
/// blocks. Loop targets, `with ... as`, walrus, and `except ... as` are not.
pub(crate) fn assignment_target_ranges(module: &ast::ModModule) -> FxHashSet<TextRange> {
    let mut out = FxHashSet::default();
    collect_assignment_targets(&module.body, &mut out);
    out
}

fn collect_assignment_targets(body: &[Stmt], out: &mut FxHashSet<TextRange>) {
    for statement in body {
        match statement {
            Stmt::Assign(assign) => {
                for target in &assign.targets {
                    collect_name_targets(target, out);
                }
            }
            Stmt::AnnAssign(assign) => collect_name_targets(&assign.target, out),
            Stmt::AugAssign(assign) => collect_name_targets(&assign.target, out),
            Stmt::TypeAlias(alias) => collect_name_targets(&alias.name, out),
            Stmt::ClassDef(class) => collect_assignment_targets(&class.body, out),
            Stmt::If(if_statement) => {
                collect_assignment_targets(&if_statement.body, out);
                for clause in &if_statement.elif_else_clauses {
                    collect_assignment_targets(&clause.body, out);
                }
            }
            Stmt::Try(try_statement) => {
                collect_assignment_targets(&try_statement.body, out);
                for handler in &try_statement.handlers {
                    let ast::ExceptHandler::ExceptHandler(handler) = handler;
                    collect_assignment_targets(&handler.body, out);
                }
                collect_assignment_targets(&try_statement.orelse, out);
                collect_assignment_targets(&try_statement.finalbody, out);
            }
            Stmt::With(with_statement) => collect_assignment_targets(&with_statement.body, out),
            _ => {}
        }
    }
}

fn collect_name_targets(target: &Expr, out: &mut FxHashSet<TextRange>) {
    match target {
        Expr::Name(name) => {
            out.insert(name.range());
        }
        Expr::Tuple(tuple) => {
            for element in &tuple.elts {
                collect_name_targets(element, out);
            }
        }
        Expr::List(list) => {
            for element in &list.elts {
                collect_name_targets(element, out);
            }
        }
        Expr::Starred(starred) => collect_name_targets(&starred.value, out),
        _ => {}
    }
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

/// The module defining what `callee` refers to, when ty can resolve it, and
/// whether that module is part of the standard library.
fn defining_module(callee: &Expr, model: &SemanticModel<'_>) -> Option<(ModulePath, Provenance)> {
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
        let module = crate::module_name_of(db, file)?;
        let top_level = module.split('.').next().unwrap_or(module.as_str());
        let minor = model.program_file().python_version(db).minor;
        let provenance = if ruff_python_stdlib::sys::is_known_standard_library(minor, top_level) {
            Provenance::StandardLibrary
        } else {
            Provenance::Package
        };
        Some((ModulePath::new(module), provenance))
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
    let (module, provenance) = defining_module(callee, model)
        .map_or((None, Provenance::Unknown), |(module, provenance)| {
            (Some(module), provenance)
        });
    Some(Decorator {
        name: dotted_name_of(callee)?,
        keywords,
        module,
        provenance,
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
