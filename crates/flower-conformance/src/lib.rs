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
            assert!(fixture_matches(&fixture), "{}", fixture.name);
        }
    }

    #[test]
    fn tampering_any_expected_fixture_value_is_detected() {
        let fixture_directory =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spec/v0.1/fixtures");
        let mut paths = fs::read_dir(fixture_directory)
            .expect("read fixture directory")
            .map(|entry| entry.expect("read fixture entry").path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>();
        paths.sort();

        for path in paths {
            let source = fs::read(&path).expect("read fixture");
            let document: serde_json::Value =
                serde_json::from_slice(&source).expect("decode fixture JSON");
            let mut expected_paths = Vec::new();
            collect_json_paths(
                &document["expected_compile"],
                "/expected_compile",
                &mut expected_paths,
            );
            for (index, step) in document["steps"]
                .as_array()
                .expect("steps must be an array")
                .iter()
                .enumerate()
            {
                collect_json_paths(
                    &step["expected"],
                    &format!("/steps/{index}/expected"),
                    &mut expected_paths,
                );
            }

            for expected_path in expected_paths {
                let mut tampered = document.clone();
                tamper_json_value(
                    tampered
                        .pointer_mut(&expected_path)
                        .expect("collected JSON path must exist"),
                );
                let detected = serde_json::from_value::<ConformanceFixture>(tampered)
                    .map_or(true, |fixture| !fixture_matches(&fixture));
                assert!(
                    detected,
                    "{} did not detect tampering at {expected_path}",
                    path.display()
                );
            }
        }
    }

    fn fixture_matches(fixture: &ConformanceFixture) -> bool {
        match (
            compile(fixture.definition.clone()),
            &fixture.expected_compile,
        ) {
            (Ok(plan), CompileExpectation::Plan { plan: expected }) => {
                ExpectedPlan::from_plan(&plan) == *expected
                    && fixture.steps.iter().all(|step| {
                        let actual = transition(&plan, step.snapshot.as_ref(), step.event.clone());
                        match (actual, &step.expected) {
                            (
                                Ok(actual),
                                TransitionExpectation::Transition { snapshot, effects },
                            ) => actual.snapshot == *snapshot && actual.effects == *effects,
                            (Err(actual), TransitionExpectation::Error { error }) => {
                                actual.code() == error.code && actual.to_string() == error.message
                            }
                            _ => false,
                        }
                    })
            }
            (
                Err(actual),
                CompileExpectation::Diagnostics {
                    diagnostics: expected,
                },
            ) => actual == *expected && fixture.steps.is_empty(),
            _ => false,
        }
    }

    fn collect_json_paths(value: &serde_json::Value, path: &str, paths: &mut Vec<String>) {
        paths.push(path.to_owned());
        match value {
            serde_json::Value::Object(fields) => {
                for (name, child) in fields {
                    let escaped_name = name.replace('~', "~0").replace('/', "~1");
                    collect_json_paths(child, &format!("{path}/{escaped_name}"), paths);
                }
            }
            serde_json::Value::Array(items) => {
                for (index, child) in items.iter().enumerate() {
                    collect_json_paths(child, &format!("{path}/{index}"), paths);
                }
            }
            _ => {}
        }
    }

    fn tamper_json_value(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Null => *value = serde_json::Value::String("tampered".to_owned()),
            serde_json::Value::Bool(current) => *current = !*current,
            serde_json::Value::Number(current) => {
                let current = current.as_u64().expect("fixture numbers are unsigned");
                *value = serde_json::Value::from(current.saturating_add(1));
            }
            serde_json::Value::String(current) => {
                if current.len() == 64 && current.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    current.replace_range(..1, if current.starts_with('0') { "1" } else { "0" });
                } else {
                    current.push_str("-tampered");
                }
            }
            serde_json::Value::Array(items) => {
                items.push(items.first().cloned().unwrap_or(serde_json::Value::Null));
            }
            serde_json::Value::Object(fields) => {
                fields.insert("__tampered".to_owned(), serde_json::Value::Bool(true));
            }
        }
    }
}
