package flower

import (
	"context"
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

func TestComponentExecutesLinearWorkflow(t *testing.T) {
	componentPath := filepath.Join(
		"..", "..", "target", "components", "flower_engine.wasm",
	)
	component, err := os.Open(componentPath)
	if err != nil {
		t.Fatalf("open %s; build the WIT component before running Go tests: %v", componentPath, err)
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

	workflow := linearWorkflow()
	transition, err := engine.Start(ctx, workflow, "seed")
	if err != nil {
		t.Fatalf("start workflow: %v", err)
	}
	assertEffects(t, transition.Effects, []ExecuteNodeEffect{{NodeID: "node1", Input: "seed"}})

	transition, err = engine.CompleteNode(ctx, workflow, transition.Snapshot, "node1", "from-node1")
	if err != nil {
		t.Fatalf("complete node1: %v", err)
	}
	assertEffects(t, transition.Effects, []ExecuteNodeEffect{{NodeID: "node2", Input: "from-node1"}})

	transition, err = engine.CompleteNode(ctx, workflow, transition.Snapshot, "node2", "final-output")
	if err != nil {
		t.Fatalf("complete node2: %v", err)
	}
	assertEffects(t, transition.Effects, []ExecuteNodeEffect{})
	if transition.Snapshot.Status != ExecutionCompleted {
		t.Fatalf("status = %v, want completed", transition.Snapshot.Status)
	}
	if transition.Snapshot.CurrentValue != "final-output" {
		t.Fatalf("output = %q, want final-output", transition.Snapshot.CurrentValue)
	}
	if !reflect.DeepEqual(transition.Snapshot.CompletedNodeIDs, []string{"node1", "node2"}) {
		t.Fatalf("completed nodes = %v, want [node1 node2]", transition.Snapshot.CompletedNodeIDs)
	}
}

func TestComponentRejectsUnexpectedCompletion(t *testing.T) {
	componentPath := filepath.Join(
		"..", "..", "target", "components", "flower_engine.wasm",
	)
	component, err := os.Open(componentPath)
	if err != nil {
		t.Fatal(err)
	}
	defer component.Close()

	ctx := context.Background()
	engine, err := LoadEngine(ctx, component)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = engine.Close(ctx) })

	workflow := linearWorkflow()
	transition, err := engine.Start(ctx, workflow, "seed")
	if err != nil {
		t.Fatal(err)
	}
	_, err = engine.CompleteNode(ctx, workflow, transition.Snapshot, "node2", "invalid")
	engineError, ok := err.(*EngineError)
	if !ok {
		t.Fatalf("error = %T %v, want *EngineError", err, err)
	}
	if engineError.Code != "transition-failed" {
		t.Fatalf("error code = %q, want transition-failed", engineError.Code)
	}
}

func linearWorkflow() WorkflowDefinition {
	return WorkflowDefinition{
		ID: "linear",
		Nodes: []NodeDefinition{
			{ID: "start", Kind: StartNode},
			{ID: "node1", Kind: ActivityNode},
			{ID: "node2", Kind: ActivityNode},
			{ID: "finish", Kind: FinishNode},
		},
		Edges: []EdgeDefinition{
			{ID: "e1", Source: "start", Target: "node1"},
			{ID: "e2", Source: "node1", Target: "node2"},
			{ID: "e3", Source: "node2", Target: "finish"},
		},
	}
}

func assertEffects(t *testing.T, actual, expected []ExecuteNodeEffect) {
	t.Helper()
	if !reflect.DeepEqual(actual, expected) {
		t.Fatalf("effects = %#v, want %#v", actual, expected)
	}
}
