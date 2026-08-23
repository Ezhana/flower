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

const fixtureSchemaVersion = "flower.conformance/v0.1"

type conformanceFixture struct {
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
	Type     string          `json:"type"`
	NodeID   string          `json:"node_id"`
	EffectID string          `json:"effect_id"`
	Input    *fixturePayload `json:"input"`
	Output   *fixturePayload `json:"output"`
}

type fixtureExecutionEvent struct {
	Type             string          `json:"type"`
	EventID          string          `json:"event_id"`
	ExecutionID      string          `json:"execution_id"`
	PlanReference    *PlanReference  `json:"plan_reference"`
	Input            *fixturePayload `json:"input"`
	ExpectedRevision *uint64         `json:"expected_revision"`
	EffectID         string          `json:"effect_id"`
	NodeID           string          `json:"node_id"`
	Output           *fixturePayload `json:"output"`
}

type fixtureExecutionEffect struct {
	Type        string         `json:"type"`
	EffectID    string         `json:"effect_id"`
	ExecutionID string         `json:"execution_id"`
	NodeID      string         `json:"node_id"`
	Input       fixturePayload `json:"input"`
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

	paths, err := filepath.Glob(filepath.Join("..", "..", "spec", "v0.1", "fixtures", "*.json"))
	if err != nil {
		t.Fatal(err)
	}
	sort.Strings(paths)
	if len(paths) == 0 {
		t.Fatal("conformance suite has no fixtures")
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

func TestCompileRejectsInvalidPublicNodeKindBeforeABI(t *testing.T) {
	engine := &componentEngine{}
	_, diagnostics, err := engine.Compile(context.Background(), WorkflowDefinition{
		ID:    "invalid-sdk-input",
		Nodes: []NodeDefinition{{ID: "bad", Kind: NodeKind("gateway")}},
	})
	if err != nil {
		t.Fatalf("Compile returned transport error: %v", err)
	}
	wantSubject := "bad"
	want := []Diagnostic{{
		Code:    "invalid-node-kind",
		Message: "node \"bad\": unsupported node kind \"gateway\"",
		Subject: &wantSubject,
	}}
	if !reflect.DeepEqual(diagnostics, want) {
		t.Fatalf("diagnostics = %#v, want %#v", diagnostics, want)
	}
}

func decodeFixture(t *testing.T, path string) conformanceFixture {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read %s: %v", path, err)
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	var fixture conformanceFixture
	if err := decoder.Decode(&fixture); err != nil {
		t.Fatalf("decode %s: %v", path, err)
	}
	var trailing any
	if err := decoder.Decode(&trailing); err != io.EOF {
		t.Fatalf("decode %s: trailing JSON value", path)
	}
	return fixture
}

func runFixture(ctx context.Context, t *testing.T, engine Engine, fixture conformanceFixture) {
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
	case "awaiting-node":
		if value.Input == nil || value.Output != nil {
			return nil, fmt.Errorf("awaiting-node state has invalid shape")
		}
		input, err := value.Input.toPublic()
		if err != nil {
			return nil, err
		}
		return AwaitingNode{NodeID: value.NodeID, EffectID: value.EffectID, Input: input}, nil
	case "completed":
		if value.Output == nil || value.Input != nil || value.NodeID != "" || value.EffectID != "" {
			return nil, fmt.Errorf("completed state has invalid shape")
		}
		output, err := value.Output.toPublic()
		if err != nil {
			return nil, err
		}
		return Completed{Output: output}, nil
	default:
		return nil, fmt.Errorf("unknown execution state %q", value.Type)
	}
}

func (value fixtureExecutionEvent) toPublic() (ExecutionEvent, error) {
	switch value.Type {
	case "execution-started":
		if value.PlanReference == nil || value.Input == nil || value.ExpectedRevision != nil || value.Output != nil || value.EffectID != "" || value.NodeID != "" {
			return nil, fmt.Errorf("execution-started event has invalid shape")
		}
		input, err := value.Input.toPublic()
		if err != nil {
			return nil, err
		}
		return ExecutionStarted{
			EventID:       value.EventID,
			ExecutionID:   value.ExecutionID,
			PlanReference: *value.PlanReference,
			Input:         input,
		}, nil
	case "node-completed":
		if value.PlanReference != nil || value.Input != nil || value.ExpectedRevision == nil || value.Output == nil {
			return nil, fmt.Errorf("node-completed event has invalid shape")
		}
		output, err := value.Output.toPublic()
		if err != nil {
			return nil, err
		}
		return NodeCompleted{
			EventID:          value.EventID,
			ExecutionID:      value.ExecutionID,
			ExpectedRevision: *value.ExpectedRevision,
			EffectID:         value.EffectID,
			NodeID:           value.NodeID,
			Output:           output,
		}, nil
	default:
		return nil, fmt.Errorf("unknown execution event %q", value.Type)
	}
}

func (value fixtureExecutionEffect) toPublic() (ExecutionEffect, error) {
	if value.Type != "execute-node" {
		return nil, fmt.Errorf("unknown execution effect %q", value.Type)
	}
	input, err := value.Input.toPublic()
	if err != nil {
		return nil, err
	}
	return ExecuteNode{
		EffectID:    value.EffectID,
		ExecutionID: value.ExecutionID,
		NodeID:      value.NodeID,
		Input:       input,
	}, nil
}
