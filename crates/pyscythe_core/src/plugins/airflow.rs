//! Airflow parses every module in the DAGs folder and instantiates decorated
//! DAGs and tasks itself.

use crate::keep::{KeepContext, KeepRule, PluginName};
use crate::plugins::decorated_with;
use crate::symbol::SymbolKind;

pub(crate) struct Airflow;

const DECORATORS: &[&str] = &["dag", "task", "task_group", "setup", "teardown"];

impl KeepRule for Airflow {
    fn plugin(&self) -> PluginName {
        PluginName::Airflow
    }

    fn keep(&self, context: KeepContext<'_>) -> Option<&'static str> {
        let symbol = context.symbol;
        let is_task_decorator = symbol.has_decorator(|d| {
            let mut segments = d.name.segments();
            let first = segments.next().unwrap_or_default();
            DECORATORS.contains(&first) || decorated_with(symbol, DECORATORS, false)
        });
        if is_task_decorator {
            return Some("Airflow DAG or task decorator");
        }
        if context.is_under_directory("dags")
            && matches!(symbol.kind, SymbolKind::Variable | SymbolKind::Constant)
        {
            return Some("module-level object in the DAGs folder");
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::Airflow;
    use crate::plugins::testing::Case;

    #[test]
    fn keeps_decorated_dags_and_tasks() {
        assert!(Case::function("etl").decorated("dag").is_kept_by(&Airflow));
        assert!(
            Case::function("extract")
                .decorated("task")
                .is_kept_by(&Airflow)
        );
        assert!(
            Case::function("branch")
                .decorated("task.branch")
                .is_kept_by(&Airflow)
        );
        assert!(
            Case::function("grp")
                .decorated("task_group")
                .is_kept_by(&Airflow)
        );
    }

    #[test]
    fn keeps_module_level_objects_in_dag_files() {
        assert!(
            Case::variable("dag")
                .at("/p/dags/etl.py")
                .is_kept_by(&Airflow)
        );
        assert!(
            !Case::function("helper")
                .at("/p/dags/etl.py")
                .is_kept_by(&Airflow)
        );
        assert!(
            !Case::variable("dag")
                .at("/p/pkg/etl.py")
                .is_kept_by(&Airflow)
        );
    }
}
