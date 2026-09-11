//! Decides whether a method overrides a member inherited from a base class,
//! walking the class hierarchy through ty so third-party and stdlib bases
//! count too. A library that calls `self.similarity_search()` never names the
//! subclass's override, so an override with no visible callers is not dead.

use pyscythe_core::source::ByteSpan;
use ruff_db::files::File;
use ruff_db::parsed::parsed_module;
use ruff_python_ast::{self as ast, Expr, Stmt};
use ruff_text_size::{Ranged, TextRange, TextSize};
use rustc_hash::FxHashSet;
use ty_python_semantic::{HasType, SemanticModel, type_hierarchy_supertypes};

/// Give up after this many classes; real hierarchies are far shallower.
const MAX_CLASSES_VISITED: usize = 64;

/// One base class reached while walking a hierarchy.
pub(crate) struct Ancestor {
    /// Qualified name such as `sqlalchemy.orm.decl_api.DeclarativeBase`.
    pub(crate) qualified_name: String,
    /// Whether this ancestor's class body defines the name being looked for.
    pub(crate) defines_name: bool,
}

/// The transitive base classes of a class.
pub(crate) struct Hierarchy {
    pub(crate) ancestors: Vec<Ancestor>,
    /// Whether every base at every level resolved to a class.
    pub(crate) complete: bool,
}

/// Whether the method named `name`, defined inside the class that encloses
/// `method_span` in `file`, overrides a member of any base class.
pub(crate) fn overrides_inherited_member(
    db: &dyn ty_project::Db,
    file: File,
    method_span: ByteSpan,
    name: &str,
) -> bool {
    hierarchy_of_enclosing_class(db, file, method_span, Some(name))
        .ancestors
        .iter()
        .any(|ancestor| ancestor.defines_name)
}

/// The hierarchy of the innermost class enclosing `span` in `file`: for a
/// method, the class that defines it.
pub(crate) fn hierarchy_of_enclosing_class(
    db: &dyn ty_project::Db,
    file: File,
    span: ByteSpan,
    looking_for: Option<&str>,
) -> Hierarchy {
    let program_file = db.program_file(file);
    let parsed = parsed_module(db, program_file.python_file(db)).load(db);
    innermost_class_enclosing(&parsed.syntax().body, to_text_range(span)).map_or_else(
        || Hierarchy {
            ancestors: Vec::new(),
            complete: false,
        },
        |class_def| walk_hierarchy(db, file, class_def, looking_for),
    )
}

/// The hierarchy of the class whose name occupies `name_span` in `file`.
pub(crate) fn hierarchy_of_class(
    db: &dyn ty_project::Db,
    file: File,
    name_span: ByteSpan,
    looking_for: Option<&str>,
) -> Hierarchy {
    let program_file = db.program_file(file);
    let parsed = parsed_module(db, program_file.python_file(db)).load(db);
    class_named_at(&parsed.syntax().body, to_text_range(name_span)).map_or_else(
        || Hierarchy {
            ancestors: Vec::new(),
            complete: false,
        },
        |class_def| walk_hierarchy(db, file, class_def, looking_for),
    )
}

const fn to_text_range(span: ByteSpan) -> TextRange {
    TextRange::new(
        TextSize::new(span.start().get()),
        TextSize::new(span.end().get()),
    )
}

/// Breadth-first over base classes, stopping early once `looking_for` is found.
fn walk_hierarchy(
    db: &dyn ty_project::Db,
    file: File,
    class_def: &ast::StmtClassDef,
    looking_for: Option<&str>,
) -> Hierarchy {
    let program_file = db.program_file(file);
    let model = SemanticModel::new(db, program_file);
    let mut hierarchy = Hierarchy {
        ancestors: Vec::new(),
        complete: true,
    };
    let Some(class_type) = class_def.inferred_type(&model) else {
        hierarchy.complete = false;
        return hierarchy;
    };

    let mut visited: FxHashSet<(File, TextRange)> = FxHashSet::default();
    let mut pending = vec![(program_file, class_type, explicit_base_count(class_def))];
    while let Some((owner_file, ty, explicit_bases)) = pending.pop() {
        if visited.len() >= MAX_CLASSES_VISITED {
            hierarchy.complete = false;
            break;
        }
        let owner_model = SemanticModel::new(db, owner_file);
        let supertypes = type_hierarchy_supertypes(db, &owner_model.program_environment(), ty);
        // ty substitutes `object` when no base resolves, so it must not count
        // towards the bases the source actually names.
        let resolved = supertypes
            .iter()
            .filter(|base| base.name.as_str() != "object")
            .count();
        if explicit_bases > 0 && resolved < explicit_bases {
            hierarchy.complete = false;
        }
        for base in supertypes {
            // `object` is every class's implicit root; its `__init_subclass__`
            // registers nothing and no rule matches on it.
            if base.name.as_str() == "object" {
                continue;
            }
            let base_file = base.file.file(db);
            if !visited.insert((base_file, base.selection_range)) {
                continue;
            }
            let base_program_file = db.program_file(base_file);
            let base_parsed = parsed_module(db, base_program_file.python_file(db)).load(db);
            let Some(base_def) = class_named_at(&base_parsed.syntax().body, base.selection_range)
            else {
                hierarchy.complete = false;
                continue;
            };
            let defines_name =
                looking_for.is_some_and(|name| class_body_defines(&base_def.body, name));
            let module = crate::module_name_of(db, base_file).unwrap_or_default();
            hierarchy.ancestors.push(Ancestor {
                qualified_name: if module.is_empty() {
                    base.name.to_string()
                } else {
                    format!("{module}.{}", base.name)
                },
                defines_name,
            });
            if defines_name {
                return hierarchy;
            }
            let base_model = SemanticModel::new(db, base_program_file);
            match base_def.inferred_type(&base_model) {
                Some(base_type) => {
                    pending.push((base_program_file, base_type, explicit_base_count(base_def)));
                }
                None => hierarchy.complete = false,
            }
        }
    }
    hierarchy
}

/// Positional bases only; `metaclass=` and `table=True` are keywords.
fn explicit_base_count(class_def: &ast::StmtClassDef) -> usize {
    class_def
        .arguments
        .as_ref()
        .map_or(0, |arguments| arguments.args.len())
}

/// The innermost `class` statement whose range contains `range`.
fn innermost_class_enclosing(body: &[Stmt], range: TextRange) -> Option<&ast::StmtClassDef> {
    for statement in body {
        let nested = match statement {
            Stmt::ClassDef(class) if class.range().contains_range(range) => {
                return innermost_class_enclosing(&class.body, range).or(Some(class));
            }
            Stmt::FunctionDef(function) => &function.body,
            Stmt::If(if_statement) => {
                if let Some(found) = innermost_class_enclosing(&if_statement.body, range) {
                    return Some(found);
                }
                for clause in &if_statement.elif_else_clauses {
                    if let Some(found) = innermost_class_enclosing(&clause.body, range) {
                        return Some(found);
                    }
                }
                continue;
            }
            Stmt::Try(try_statement) => &try_statement.body,
            Stmt::With(with_statement) => &with_statement.body,
            _ => continue,
        };
        if let Some(found) = innermost_class_enclosing(nested, range) {
            return Some(found);
        }
    }
    None
}

/// The `class` statement whose name occupies `name_range`, at any nesting depth.
fn class_named_at(body: &[Stmt], name_range: TextRange) -> Option<&ast::StmtClassDef> {
    for statement in body {
        let nested = match statement {
            Stmt::ClassDef(class) => {
                if class.name.range() == name_range {
                    return Some(class);
                }
                &class.body
            }
            Stmt::FunctionDef(function) => &function.body,
            Stmt::If(if_statement) => {
                if let Some(found) = class_named_at(&if_statement.body, name_range) {
                    return Some(found);
                }
                for clause in &if_statement.elif_else_clauses {
                    if let Some(found) = class_named_at(&clause.body, name_range) {
                        return Some(found);
                    }
                }
                continue;
            }
            Stmt::Try(try_statement) => &try_statement.body,
            Stmt::With(with_statement) => &with_statement.body,
            _ => continue,
        };
        if let Some(found) = class_named_at(nested, name_range) {
            return Some(found);
        }
    }
    None
}

/// Whether a class body defines `name` as a method, nested class, or attribute.
fn class_body_defines(body: &[Stmt], name: &str) -> bool {
    body.iter().any(|statement| match statement {
        Stmt::FunctionDef(function) => function.name.as_str() == name,
        Stmt::ClassDef(class) => class.name.as_str() == name,
        Stmt::AnnAssign(assign) => target_is(&assign.target, name),
        Stmt::Assign(assign) => assign.targets.iter().any(|t| target_is(t, name)),
        Stmt::If(if_statement) => {
            class_body_defines(&if_statement.body, name)
                || if_statement
                    .elif_else_clauses
                    .iter()
                    .any(|clause| class_body_defines(&clause.body, name))
        }
        _ => false,
    })
}

fn target_is(target: &Expr, name: &str) -> bool {
    matches!(target, Expr::Name(id) if id.id.as_str() == name)
}
