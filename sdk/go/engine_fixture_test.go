package flower

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"sort"
	"testing"
)

const fixtureSchemaVersion = "flower.engine-fixture/v1"

type engineFixture struct {
	SchemaVersion   string                    `json:"schema_version"`
	Name            string                    `json:"name"`
	Definition      WorkflowDefinition        `json:"definition"`
	ExpectedCompile fixtureCompileExpectation `json:"expected_compile"`
	Steps           []fixtureTransitionStep   `json:"steps"`
}

type fixtureCompileExpectation struct {
	Outcome     string                  `json:"outcome"`
	Plan        *ExecutableWorkflowPlan `json:"plan"`
	Diagnostics []Diagnostic            `json:"diagnostics"`
}

type fixtureTransitionStep struct {
	Snapshot *fixtureExecutionSnapshot    `json:"snapshot"`
	Event    fixtureExecutionEvent        `json:"event"`
	Expected fixtureTransitionExpectation `json:"expected"`
}

type fixtureTransitionExpectation struct {
	Outcome  string                    `json:"outcome"`
	Snapshot *fixtureExecutionSnapshot `json:"snapshot"`
	Effects  []fixtureExecutionEffect  `json:"effects"`
	Error    *EngineError              `json:"error"`
}

type fixturePayload struct {
	MediaType string `json:"media_type"`
	Bytes     []int  `json:"bytes"`
}

type fixtureExecutionSnapshot struct {
	ExecutionID   string                `json:"execution_id"`
	PlanReference PlanReference         `json:"plan_reference"`
	Revision      uint64                `json:"revision"`
	State         fixtureExecutionState `json:"state"`
}

type fixtureExecutionState struct {
	Type       string                 `json:"type"`
	Activation *fixtureNodeActivation `json:"activation"`
	Attempt    *fixtureNodeAttempt    `json:"attempt"`
	Failure    *fixtureAttemptFailure `json:"failure"`
	Timer      *fixtureRetryTimer     `json:"timer"`
	Output     *fixturePayload        `json:"output"`
}

type fixtureNodeActivation struct {
	ActivationID       string         `json:"activation_id"`
	ActivationRevision uint64         `json:"activation_revision"`
	NodeID             string         `json:"node_id"`
	Input              fixturePayload `json:"input"`
}

type fixtureNodeAttempt struct {
	AttemptID     string `json:"attempt_id"`
	AttemptNumber uint32 `json:"attempt_number"`
	EffectID      string `json:"effect_id"`
}

type fixtureAttemptFailure struct {
	Code    string          `json:"code"`
	Details *fixturePayload `json:"details"`
}

type fixtureRetryTimer struct {
	TimerID           string `json:"timer_id"`
	EffectID          string `json:"effect_id"`
	FailedAttemptID   string `json:"failed_attempt_id"`
	NextAttemptNumber uint32 `json:"next_attempt_number"`
	DelayMs           uint64 `json:"delay_ms"`
}

type fixtureExecutionEvent struct {
	Type              string                 `json:"type"`
	ExecutionID       string                 `json:"execution_id"`
	PlanReference     *PlanReference         `json:"plan_reference"`
	Input             *fixturePayload        `json:"input"`
	ExpectedRevision  *uint64                `json:"expected_revision"`
	ActivationID      string                 `json:"activation_id"`
	AttemptID         string                 `json:"attempt_id"`
	AttemptNumber     *uint32                `json:"attempt_number"`
	EffectID          string                 `json:"effect_id"`
	NodeID            string                 `json:"node_id"`
	Output            *fixturePayload        `json:"output"`
	Failure           *fixtureAttemptFailure `json:"failure"`
	TimerID           string                 `json:"timer_id"`
	NextAttemptNumber *uint32                `json:"next_attempt_number"`
}

type fixtureExecutionEffect struct {
	Type              string         `json:"type"`
	EffectID          string         `json:"effect_id"`
	ActivationID      string         `json:"activation_id"`
	AttemptID         string         `json:"attempt_id"`
	AttemptNumber     uint32         `json:"attempt_number"`
	TimerID           string         `json:"timer_id"`
	FailedAttemptID   string         `json:"failed_attempt_id"`
	NextAttemptNumber uint32         `json:"next_attempt_number"`
	DelayMs           uint64         `json:"delay_ms"`
	NodeID            string         `json:"node_id"`
	Input             fixturePayload `json:"input"`
}

func TestComponentExecutesEverySharedFixtureExpectation(t *testing.T) {
	component, err := os.Open(filepath.Join("..", "..", "target", "components", "flower_engine.wasm"))
	if err != nil {
		t.Fatalf("open component: %v", err)
	}
	defer component.Close()
	ctx := context.Background()
	engine, err := LoadEngine(ctx, component)
	if err != nil {
		t.Fatalf("load engine: %v", err)
	}
	t.Cleanup(func() {
		if err := engine.Close(ctx); err != nil {
			t.Errorf("close engine: %v", err)
		}
	})

	paths, err := filepath.Glob(filepath.Join("..", "..", "fixtures", "cases", "*.json"))
	if err != nil {
		t.Fatal(err)
	}
	sort.Strings(paths)
	if len(paths) == 0 {
		t.Fatal("engine fixture suite has no fixtures")
	}
	names := map[string]bool{}
	for _, path := range paths {
		current := decodeFixture(t, path)
		if current.SchemaVersion != fixtureSchemaVersion {
			t.Fatalf("%s schema version = %q", path, current.SchemaVersion)
		}
		if names[current.Name] {
			t.Fatalf("duplicate fixture name %q", current.Name)
		}
		names[current.Name] = true
		t.Run(current.Name, func(t *testing.T) {
			runFixture(ctx, t, engine, current)
		})
	}
}

func TestCompileReturnsPublicToABIConversionErrors(t *testing.T) {
	engine := &componentEngine{}
	_, diagnostics, err := engine.Compile(context.Background(), WorkflowDefinition{
		ID:    "invalid-sdk-input",
		Nodes: []NodeDefinition{{ID: "bad", Kind: NodeKind("gateway")}},
	})
	if err == nil || err.Error() != `convert definition.nodes[0] "bad": unsupported node kind "gateway"` {
		t.Fatalf("Compile error = %v", err)
	}
	if diagnostics != nil {
		t.Fatalf("diagnostics = %#v, want nil", diagnostics)
	}
}

func TestTamperingAnyExpectedJSONValueChangesTheTypedExpectation(t *testing.T) {
	paths, err := filepath.Glob(filepath.Join("..", "..", "fixtures", "cases", "*.json"))
	if err != nil {
		t.Fatal(err)
	}
	sort.Strings(paths)
	for _, path := range paths {
		source, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		original, err := decodeFixtureBytes(source)
		if err != nil {
			t.Fatalf("decode %s: %v", path, err)
		}
		var document map[string]any
		if err := json.Unmarshal(source, &document); err != nil {
			t.Fatal(err)
		}
		var expectedPaths [][]any
		collectJSONPaths(document["expected_compile"], []any{"expected_compile"}, &expectedPaths)
		for index, step := range document["steps"].([]any) {
			collectJSONPaths(step.(map[string]any)["expected"], []any{"steps", index, "expected"}, &expectedPaths)
		}

		for _, expectedPath := range expectedPaths {
			clonedBytes, err := json.Marshal(document)
			if err != nil {
				t.Fatal(err)
			}
			var tamperedDocument map[string]any
			if err := json.Unmarshal(clonedBytes, &tamperedDocument); err != nil {
				t.Fatal(err)
			}
			if _, err := tamperJSONAtPath(tamperedDocument, expectedPath); err != nil {
				t.Fatalf("tamper %s at %v: %v", path, expectedPath, err)
			}
			tamperedBytes, err := json.Marshal(tamperedDocument)
			if err != nil {
				t.Fatal(err)
			}
			tampered, err := decodeFixtureBytes(tamperedBytes)
			if err == nil && reflect.DeepEqual(tampered.ExpectedCompile, original.ExpectedCompile) && reflect.DeepEqual(tampered.Steps, original.Steps) {
				t.Fatalf("%s did not retain tampering at %v", path, expectedPath)
			}
		}
	}
}

func decodeFixture(t *testing.T, path string) engineFixture {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read %s: %v", path, err)
	}
	fixture, err := decodeFixtureBytes(data)
	if err != nil {
		t.Fatalf("decode %s: %v", path, err)
	}
	return fixture
}

func decodeFixtureBytes(data []byte) (engineFixture, error) {
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	var fixture engineFixture
	if err := decoder.Decode(&fixture); err != nil {
		return engineFixture{}, err
	}
	var trailing any
	if err := decoder.Decode(&trailing); err != io.EOF {
		return engineFixture{}, fmt.Errorf("trailing JSON value")
	}
	return fixture, nil
}

func collectJSONPaths(value any, path []any, paths *[][]any) {
	*paths = append(*paths, append([]any(nil), path...))
	switch current := value.(type) {
	case map[string]any:
		names := make([]string, 0, len(current))
		for name := range current {
			names = append(names, name)
		}
		sort.Strings(names)
		for _, name := range names {
			collectJSONPaths(current[name], append(path, name), paths)
		}
	case []any:
		for index, child := range current {
			collectJSONPaths(child, append(path, index), paths)
		}
	}
}

func tamperJSONAtPath(value any, path []any) (any, error) {
	if len(path) == 0 {
		return tamperJSONValue(value), nil
	}
	switch segment := path[0].(type) {
	case string:
		object, ok := value.(map[string]any)
		if !ok {
			return nil, fmt.Errorf("expected object before field %q", segment)
		}
		child, exists := object[segment]
		if !exists {
			return nil, fmt.Errorf("field %q does not exist", segment)
		}
		mutated, err := tamperJSONAtPath(child, path[1:])
		if err != nil {
			return nil, err
		}
		object[segment] = mutated
		return object, nil
	case int:
		array, ok := value.([]any)
		if !ok || segment < 0 || segment >= len(array) {
			return nil, fmt.Errorf("array index %d does not exist", segment)
		}
		mutated, err := tamperJSONAtPath(array[segment], path[1:])
		if err != nil {
			return nil, err
		}
		array[segment] = mutated
		return array, nil
	default:
		return nil, fmt.Errorf("unsupported path segment %T", segment)
	}
}

func tamperJSONValue(value any) any {
	switch current := value.(type) {
	case nil:
		return "tampered"
	case bool:
		return !current
	case float64:
		return current + 1
	case string:
		if len(current) == 64 {
			if current[0] == '0' {
				return "1" + current[1:]
			}
			return "0" + current[1:]
		}
		return current + "-tampered"
	case []any:
		if len(current) == 0 {
			return append(current, nil)
		}
		return append(current, current[0])
	case map[string]any:
		current["__tampered"] = true
		return current
	default:
		panic(fmt.Sprintf("unsupported JSON value %T", value))
	}
}

func runFixture(ctx context.Context, t *testing.T, engine Engine, fixture engineFixture) {
	t.Helper()
	plan, diagnostics, err := engine.Compile(ctx, fixture.Definition)
	if err != nil {
		t.Fatalf("compile call: %v", err)
	}
	switch fixture.ExpectedCompile.Outcome {
	case "plan":
		if fixture.ExpectedCompile.Plan == nil || fixture.ExpectedCompile.Diagnostics != nil {
			t.Fatal("plan expectation has invalid shape")
		}
		if len(diagnostics) != 0 {
			t.Fatalf("diagnostics = %#v, want none", diagnostics)
		}
		if !reflect.DeepEqual(plan, *fixture.ExpectedCompile.Plan) {
			t.Fatalf("plan = %#v, want %#v", plan, *fixture.ExpectedCompile.Plan)
		}
		for index, step := range fixture.Steps {
			runTransitionStep(ctx, t, engine, plan, index, step)
		}
	case "diagnostics":
		if fixture.ExpectedCompile.Plan != nil || fixture.ExpectedCompile.Diagnostics == nil {
			t.Fatal("diagnostics expectation has invalid shape")
		}
		if !reflect.DeepEqual(diagnostics, fixture.ExpectedCompile.Diagnostics) {
			t.Fatalf("diagnostics = %#v, want %#v", diagnostics, fixture.ExpectedCompile.Diagnostics)
		}
		if len(fixture.Steps) != 0 {
			t.Fatal("fixture declares transitions without a compiled plan")
		}
	default:
		t.Fatalf("unknown compile outcome %q", fixture.ExpectedCompile.Outcome)
	}
}

func runTransitionStep(
	ctx context.Context,
	t *testing.T,
	engine Engine,
	plan ExecutableWorkflowPlan,
	index int,
	step fixtureTransitionStep,
) {
	t.Helper()
	var snapshot *ExecutionSnapshot
	if step.Snapshot != nil {
		converted, err := step.Snapshot.toPublic()
		if err != nil {
			t.Fatalf("step %d snapshot: %v", index, err)
		}
		snapshot = &converted
	}
	event, err := step.Event.toPublic()
	if err != nil {
		t.Fatalf("step %d event: %v", index, err)
	}
	actual, actualError := engine.Transition(ctx, plan, snapshot, event)

	switch step.Expected.Outcome {
	case "transition":
		if step.Expected.Snapshot == nil || step.Expected.Error != nil {
			t.Fatalf("step %d transition expectation has invalid shape", index)
		}
		if actualError != nil {
			t.Fatalf("step %d transition: %v", index, actualError)
		}
		expectedSnapshot, err := step.Expected.Snapshot.toPublic()
		if err != nil {
			t.Fatalf("step %d expected snapshot: %v", index, err)
		}
		expectedEffects := make([]ExecutionEffect, len(step.Expected.Effects))
		for effectIndex, effect := range step.Expected.Effects {
			expectedEffects[effectIndex], err = effect.toPublic()
			if err != nil {
				t.Fatalf("step %d effect %d: %v", index, effectIndex, err)
			}
		}
		if !reflect.DeepEqual(actual.Snapshot, expectedSnapshot) {
			t.Fatalf("step %d snapshot = %#v, want %#v", index, actual.Snapshot, expectedSnapshot)
		}
		if !reflect.DeepEqual(actual.Effects, expectedEffects) {
			t.Fatalf("step %d effects = %#v, want %#v", index, actual.Effects, expectedEffects)
		}
	case "error":
		if step.Expected.Error == nil || step.Expected.Snapshot != nil || step.Expected.Effects != nil {
			t.Fatalf("step %d error expectation has invalid shape", index)
		}
		engineError, ok := actualError.(*EngineError)
		if !ok {
			t.Fatalf("step %d error = %T %v, want *EngineError", index, actualError, actualError)
		}
		if *engineError != *step.Expected.Error {
			t.Fatalf("step %d error = %#v, want %#v", index, engineError, step.Expected.Error)
		}
	default:
		t.Fatalf("step %d unknown outcome %q", index, step.Expected.Outcome)
	}
}

func (value fixturePayload) toPublic() (Payload, error) {
	bytes := make([]byte, len(value.Bytes))
	for index, item := range value.Bytes {
		if item < 0 || item > 255 {
			return Payload{}, fmt.Errorf("byte %d is outside u8", item)
		}
		bytes[index] = byte(item)
	}
	return Payload{MediaType: value.MediaType, Bytes: bytes}, nil
}

func (value fixtureExecutionSnapshot) toPublic() (ExecutionSnapshot, error) {
	state, err := value.State.toPublic()
	if err != nil {
		return ExecutionSnapshot{}, err
	}
	return ExecutionSnapshot{
		ExecutionID:   value.ExecutionID,
		PlanReference: value.PlanReference,
		Revision:      value.Revision,
		State:         state,
	}, nil
}

func (value fixtureExecutionState) toPublic() (ExecutionState, error) {
	switch value.Type {
	case "awaiting-attempt":
		if value.Activation == nil || value.Attempt == nil || value.Failure != nil || value.Timer != nil || value.Output != nil {
			return nil, fmt.Errorf("awaiting-attempt state has invalid shape")
		}
		activation, err := value.Activation.toPublic()
		if err != nil {
			return nil, err
		}
		return AwaitingAttempt{Activation: activation, Attempt: value.Attempt.toPublic()}, nil
	case "completed":
		if value.Output == nil || value.Activation != nil || value.Attempt != nil || value.Failure != nil || value.Timer != nil {
			return nil, fmt.Errorf("completed state has invalid shape")
		}
		output, err := value.Output.toPublic()
		if err != nil {
			return nil, err
		}
		return Completed{Output: output}, nil
	case "failed":
		if value.Activation == nil || value.Attempt == nil || value.Failure == nil || value.Timer != nil || value.Output != nil {
			return nil, fmt.Errorf("failed state has invalid shape")
		}
		activation, err := value.Activation.toPublic()
		if err != nil {
			return nil, err
		}
		failure, err := value.Failure.toPublic()
		if err != nil {
			return nil, err
		}
		return Failed{Activation: activation, Attempt: value.Attempt.toPublic(), Failure: failure}, nil
	case "waiting-for-retry":
		if value.Activation == nil || value.Attempt == nil || value.Failure == nil || value.Timer == nil || value.Output != nil {
			return nil, fmt.Errorf("waiting-for-retry state has invalid shape")
		}
		activation, err := value.Activation.toPublic()
		if err != nil {
			return nil, err
		}
		failure, err := value.Failure.toPublic()
		if err != nil {
			return nil, err
		}
		return WaitingForRetry{
			Activation: activation, Attempt: value.Attempt.toPublic(), Failure: failure,
			Timer: value.Timer.toPublic(),
		}, nil
	default:
		return nil, fmt.Errorf("unknown execution state %q", value.Type)
	}
}

func (value fixtureRetryTimer) toPublic() RetryTimer {
	return RetryTimer{
		TimerID: value.TimerID, EffectID: value.EffectID, FailedAttemptID: value.FailedAttemptID,
		NextAttemptNumber: value.NextAttemptNumber, DelayMs: value.DelayMs,
	}
}

func (value fixtureNodeActivation) toPublic() (NodeActivation, error) {
	input, err := value.Input.toPublic()
	if err != nil {
		return NodeActivation{}, err
	}
	return NodeActivation{
		ActivationID:       value.ActivationID,
		ActivationRevision: value.ActivationRevision,
		NodeID:             value.NodeID,
		Input:              input,
	}, nil
}

func (value fixtureNodeAttempt) toPublic() NodeAttempt {
	return NodeAttempt{
		AttemptID:     value.AttemptID,
		AttemptNumber: value.AttemptNumber,
		EffectID:      value.EffectID,
	}
}

func (value fixtureAttemptFailure) toPublic() (AttemptFailure, error) {
	var details *Payload
	if value.Details != nil {
		converted, err := value.Details.toPublic()
		if err != nil {
			return AttemptFailure{}, err
		}
		details = &converted
	}
	return AttemptFailure{Code: value.Code, Details: details}, nil
}

func (value fixtureExecutionEvent) toPublic() (ExecutionEvent, error) {
	switch value.Type {
	case "execution-started":
		if value.PlanReference == nil || value.Input == nil || value.ExpectedRevision != nil || value.AttemptNumber != nil || value.NextAttemptNumber != nil || value.Output != nil || value.Failure != nil || value.ActivationID != "" || value.AttemptID != "" || value.EffectID != "" || value.TimerID != "" || value.NodeID != "" {
			return nil, fmt.Errorf("execution-started event has invalid shape")
		}
		input, err := value.Input.toPublic()
		if err != nil {
			return nil, err
		}
		return ExecutionStarted{
			ExecutionID:   value.ExecutionID,
			PlanReference: *value.PlanReference,
			Input:         input,
		}, nil
	case "node-attempt-succeeded":
		if value.PlanReference != nil || value.Input != nil || value.ExpectedRevision == nil || value.AttemptNumber == nil || value.Output == nil || value.Failure != nil {
			return nil, fmt.Errorf("node-attempt-succeeded event has invalid shape")
		}
		output, err := value.Output.toPublic()
		if err != nil {
			return nil, err
		}
		return NodeAttemptSucceeded{
			ExecutionID:      value.ExecutionID,
			ExpectedRevision: *value.ExpectedRevision,
			ActivationID:     value.ActivationID,
			AttemptID:        value.AttemptID,
			AttemptNumber:    *value.AttemptNumber,
			EffectID:         value.EffectID,
			NodeID:           value.NodeID,
			Output:           output,
		}, nil
	case "node-attempt-failed":
		if value.PlanReference != nil || value.Input != nil || value.ExpectedRevision == nil || value.AttemptNumber == nil || value.Output != nil || value.Failure == nil {
			return nil, fmt.Errorf("node-attempt-failed event has invalid shape")
		}
		failure, err := value.Failure.toPublic()
		if err != nil {
			return nil, err
		}
		return NodeAttemptFailed{
			ExecutionID:      value.ExecutionID,
			ExpectedRevision: *value.ExpectedRevision,
			ActivationID:     value.ActivationID,
			AttemptID:        value.AttemptID,
			AttemptNumber:    *value.AttemptNumber,
			EffectID:         value.EffectID,
			NodeID:           value.NodeID,
			Failure:          failure,
		}, nil
	case "timer-fired":
		if value.PlanReference != nil || value.Input != nil || value.ExpectedRevision == nil || value.AttemptNumber != nil || value.NextAttemptNumber == nil || value.Output != nil || value.Failure != nil || value.AttemptID != "" || value.EffectID != "" || value.NodeID != "" {
			return nil, fmt.Errorf("timer-fired event has invalid shape")
		}
		return TimerFired{
			ExecutionID:      value.ExecutionID,
			ExpectedRevision: *value.ExpectedRevision, TimerID: value.TimerID,
			ActivationID: value.ActivationID, NextAttemptNumber: *value.NextAttemptNumber,
		}, nil
	default:
		return nil, fmt.Errorf("unknown execution event %q", value.Type)
	}
}

func (value fixtureExecutionEffect) toPublic() (ExecutionEffect, error) {
	switch value.Type {
	case "execute-node-attempt":
		input, err := value.Input.toPublic()
		if err != nil {
			return nil, err
		}
		return ExecuteNodeAttempt{
			EffectID: value.EffectID, ActivationID: value.ActivationID,
			AttemptID: value.AttemptID, AttemptNumber: value.AttemptNumber,
			NodeID: value.NodeID, Input: input,
		}, nil
	case "schedule-timer":
		return ScheduleTimer{
			EffectID: value.EffectID, TimerID: value.TimerID, ActivationID: value.ActivationID,
			FailedAttemptID: value.FailedAttemptID, NextAttemptNumber: value.NextAttemptNumber,
			DelayMs: value.DelayMs,
		}, nil
	default:
		return nil, fmt.Errorf("unknown execution effect %q", value.Type)
	}
}
