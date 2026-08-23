//! Versioned, language-neutral conformance fixture schema.

use flower_compiler::{Diagnostic, WorkflowDefinition};
use flower_kernel::{ExecutionEffect, ExecutionEvent, ExecutionSnapshot, Transition};
use flower_plan::{
    ExecutableWorkflowPlan, PlanFingerprint, PlanNode, SpecificationVersion, WorkflowId,
};
use serde::{Deserialize, Serialize};

pub const FIXTURE_SCHEMA_VERSION: &str = "flower.conformance/v0.1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConformanceFixture {
    pub schema_version: String,
    pub name: String,
    pub definition: WorkflowDefinition,
    pub expected_compile: CompileExpectation,
    pub steps: Vec<TransitionStep>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CompileExpectation {
    Plan { plan: ExpectedPlan },
    Diagnostics { diagnostics: Vec<Diagnostic> },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedPlan {
    pub specification_version: SpecificationVersion,
    pub workflow_id: WorkflowId,
    pub fingerprint: PlanFingerprint,
    pub nodes: Vec<PlanNode>,
}

impl ExpectedPlan {
    pub fn from_plan(plan: &ExecutableWorkflowPlan) -> Self {
        Self {
            specification_version: plan.specification_version(),
            workflow_id: plan.workflow_id().clone(),
            fingerprint: plan.fingerprint().clone(),
            nodes: plan.nodes().to_vec(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionStep {
    pub snapshot: Option<ExecutionSnapshot>,
    pub event: ExecutionEvent,
    pub expected: TransitionExpectation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "kebab-case", deny_unknown_fields)]
pub enum TransitionExpectation {
    Transition {
        snapshot: ExecutionSnapshot,
        effects: Vec<ExecutionEffect>,
    },
    Error {
        error: ExpectedEngineError,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedEngineError {
    pub code: String,
    pub message: String,
}

impl From<Transition> for TransitionExpectation {
    fn from(value: Transition) -> Self {
        Self::Transition {
            snapshot: value.snapshot,
            effects: value.effects,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, fs, path::PathBuf};

    use flower_compiler::compile;
    use flower_kernel::transition;

    use super::*;

    #[test]
    fn every_v0_1_fixture_executes_its_declared_expectations() {
        let specification_directory =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spec/v0.1");
        serde_json::from_slice::<serde_json::Value>(
            &fs::read(specification_directory.join("fixture-schema.json"))
                .expect("read fixture JSON Schema"),
        )
        .expect("fixture JSON Schema is valid JSON");
        let directory = specification_directory.join("fixtures");
        let mut paths = fs::read_dir(directory)
            .expect("read fixture directory")
            .map(|entry| entry.expect("read fixture entry").path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>();
        paths.sort();
        assert!(!paths.is_empty(), "conformance suite has no fixtures");

        let mut names = BTreeSet::new();
        for path in paths {
            let fixture: ConformanceFixture = serde_json::from_slice(
                &fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display())),
            )
            .unwrap_or_else(|error| panic!("decode {}: {error}", path.display()));
            assert_eq!(
                fixture.schema_version, FIXTURE_SCHEMA_VERSION,
                "{}",
                fixture.name
            );
            assert!(
                names.insert(fixture.name.clone()),
                "duplicate fixture name {}",
                fixture.name
            );
            run_fixture(fixture);
        }
    }

    fn run_fixture(fixture: ConformanceFixture) {
        match (compile(fixture.definition), fixture.expected_compile) {
            (Ok(actual), CompileExpectation::Plan { plan: expected }) => {
                assert_eq!(
                    ExpectedPlan::from_plan(&actual),
                    expected,
                    "{} compile plan",
                    fixture.name
                );
                for (index, step) in fixture.steps.into_iter().enumerate() {
                    let actual = transition(&actual, step.snapshot.as_ref(), step.event);
                    match (actual, step.expected) {
                        (Ok(actual), TransitionExpectation::Transition { snapshot, effects }) => {
                            assert_eq!(
                                actual.snapshot, snapshot,
                                "{} step {index} snapshot",
                                fixture.name
                            );
                            assert_eq!(
                                actual.effects, effects,
                                "{} step {index} effects",
                                fixture.name
                            );
                        }
                        (Err(actual), TransitionExpectation::Error { error }) => {
                            assert_eq!(
                                actual.code(),
                                error.code,
                                "{} step {index} error code",
                                fixture.name
                            );
                            assert_eq!(
                                actual.to_string(),
                                error.message,
                                "{} step {index} error message",
                                fixture.name
                            );
                        }
                        (actual, expected) => panic!(
                            "{} step {index}: actual {actual:?} does not match {expected:?}",
                            fixture.name
                        ),
                    }
                }
            }
            (
                Err(actual),
                CompileExpectation::Diagnostics {
                    diagnostics: expected,
                },
            ) => {
                assert_eq!(actual, expected, "{} compile diagnostics", fixture.name);
                assert!(
                    fixture.steps.is_empty(),
                    "{} cannot transition without a plan",
                    fixture.name
                );
            }
            (actual, expected) => panic!(
                "{} compile result {actual:?} does not match {expected:?}",
                fixture.name
            ),
        }
    }
}
