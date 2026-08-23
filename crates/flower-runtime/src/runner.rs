use flower_ir::{NodeId, Workflow};
use thiserror::Error;

use crate::{ExecutionEffect, ExecutionEvent, ExecutionStatus, TransitionError, WorkflowEngine};

pub trait NodeExecutor {
    type Error;

    fn execute(&mut self, node_id: &NodeId, input: &str) -> Result<String, Self::Error>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionReport {
    pub output: String,
    pub completed_node_ids: Vec<NodeId>,
}

#[derive(Debug, Error)]
pub enum RunError<ExecutorError> {
    #[error("workflow transition failed: {0}")]
    Transition(#[from] TransitionError),
    #[error("node execution failed")]
    NodeExecution(ExecutorError),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WorkflowRunner {
    engine: WorkflowEngine,
}

impl WorkflowRunner {
    pub fn run<Executor: NodeExecutor>(
        &self,
        workflow: &Workflow,
        input: impl Into<String>,
        executor: &mut Executor,
    ) -> Result<ExecutionReport, RunError<Executor::Error>> {
        let mut transition = self.engine.start(workflow, input)?;
        loop {
            if transition.snapshot().status() == ExecutionStatus::Completed {
                return Ok(ExecutionReport {
                    output: transition.snapshot().current_value().to_owned(),
                    completed_node_ids: transition.snapshot().completed_node_ids().to_vec(),
                });
            }

            let effect = transition
                .effects()
                .first()
                .expect("running linear execution emits exactly one effect");
            let ExecutionEffect::ExecuteNode { node_id, input } = effect;
            let completed_node_id = node_id.clone();
            let output = executor
                .execute(node_id, input)
                .map_err(RunError::NodeExecution)?;
            transition = self.engine.apply(
                workflow,
                transition.into_snapshot(),
                ExecutionEvent::NodeCompleted {
                    node_id: completed_node_id,
                    output,
                },
            )?;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use flower_ir::{
        EdgeDefinition, EdgeId, NodeDefinition, NodeKind, WorkflowDefinition, WorkflowId,
    };

    use super::*;

    struct RecordingExecutor {
        calls: Vec<(NodeId, String)>,
    }

    impl NodeExecutor for RecordingExecutor {
        type Error = Infallible;

        fn execute(&mut self, node_id: &NodeId, input: &str) -> Result<String, Self::Error> {
            self.calls.push((node_id.clone(), input.to_owned()));
            Ok(format!("{input}|{node_id}"))
        }
    }

    fn linear_workflow() -> Workflow {
        let node = |id, kind| NodeDefinition {
            id: NodeId::new(id).unwrap(),
            kind,
        };
        let edge = |id, source, target| EdgeDefinition {
            id: EdgeId::new(id).unwrap(),
            source: NodeId::new(source).unwrap(),
            target: NodeId::new(target).unwrap(),
        };
        Workflow::try_from(WorkflowDefinition {
            id: WorkflowId::new("linear").unwrap(),
            nodes: vec![
                node("start", NodeKind::Start),
                node("node1", NodeKind::Activity),
                node("node2", NodeKind::Activity),
                node("finish", NodeKind::Finish),
            ],
            edges: vec![
                edge("e1", "start", "node1"),
                edge("e2", "node1", "node2"),
                edge("e3", "node2", "finish"),
            ],
        })
        .unwrap()
    }

    #[test]
    fn executes_start_node1_node2_finish_in_order() {
        let mut executor = RecordingExecutor { calls: Vec::new() };
        let report = WorkflowRunner::default()
            .run(&linear_workflow(), "input", &mut executor)
            .unwrap();

        assert_eq!(
            executor.calls,
            vec![
                (NodeId::new("node1").unwrap(), "input".to_owned()),
                (NodeId::new("node2").unwrap(), "input|node1".to_owned()),
            ]
        );
        assert_eq!(report.output, "input|node1|node2");
        assert_eq!(
            report.completed_node_ids,
            vec![NodeId::new("node1").unwrap(), NodeId::new("node2").unwrap()]
        );
    }
}
