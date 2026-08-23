use flower_ir::{Workflow, WorkflowDefinition, WorkflowValidationError};

/// Converts an input-facing workflow definition into validated runtime IR.
#[derive(Clone, Copy, Debug, Default)]
pub struct WorkflowCompiler;

impl WorkflowCompiler {
    pub fn compile(
        &self,
        definition: WorkflowDefinition,
    ) -> Result<Workflow, WorkflowValidationError> {
        Workflow::try_from(definition)
    }
}
