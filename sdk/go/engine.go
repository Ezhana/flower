package flower

import (
	"context"
	"fmt"
	"io"
	"sync"

	"github.com/partite-ai/wacogo"
)

type NodeKind uint8

const (
	StartNode NodeKind = iota
	ActivityNode
	FinishNode
)

type NodeDefinition struct {
	ID   string
	Kind NodeKind
}

type EdgeDefinition struct {
	ID     string
	Source string
	Target string
}

type WorkflowDefinition struct {
	ID    string
	Nodes []NodeDefinition
	Edges []EdgeDefinition
}

type ExecutionStatus uint8

const (
	ExecutionRunning ExecutionStatus = iota
	ExecutionCompleted
)

type ExecutionSnapshot struct {
	WorkflowID       string
	Status           ExecutionStatus
	PendingNodeID    *string
	CurrentValue     string
	CompletedNodeIDs []string
}

type ExecuteNodeEffect struct {
	NodeID string
	Input  string
}

type Transition struct {
	Snapshot ExecutionSnapshot
	Effects  []ExecuteNodeEffect
}

type EngineError struct {
	Code    string
	Message string
}

func (e *EngineError) Error() string {
	return fmt.Sprintf("flower engine %s: %s", e.Code, e.Message)
}

// Engine owns one non-reentrant Component instance and serializes calls to it.
type Engine struct {
	runtime  *wacogo.Engine
	instance *wacogo.ComponentInstance
	api      *wacogo.ComponentInstance
	mu       sync.Mutex
}

func LoadEngine(ctx context.Context, component io.Reader) (*Engine, error) {
	runtime := wacogo.NewEngine(ctx)
	compiled, err := runtime.LoadComponent(ctx, component)
	if err != nil {
		_ = runtime.Close(ctx)
		return nil, fmt.Errorf("load flower component: %w", err)
	}
	instance, err := compiled.Instantiate(ctx)
	if err != nil {
		_ = runtime.Close(ctx)
		return nil, fmt.Errorf("instantiate flower component: %w", err)
	}
	api := instance.ExportedInstance("flower:engine/workflow-engine@0.1.0")
	if api == nil {
		_ = instance.Close(ctx)
		_ = runtime.Close(ctx)
		return nil, fmt.Errorf("flower component does not export workflow-engine@0.1.0")
	}
	return &Engine{runtime: runtime, instance: instance, api: api}, nil
}

func (e *Engine) Close(ctx context.Context) error {
	e.mu.Lock()
	defer e.mu.Unlock()
	if e.instance == nil {
		return nil
	}
	instanceErr := e.instance.Close(ctx)
	runtimeErr := e.runtime.Close(ctx)
	e.instance = nil
	if instanceErr != nil {
		return fmt.Errorf("close flower component instance: %w", instanceErr)
	}
	if runtimeErr != nil {
		return fmt.Errorf("close flower component runtime: %w", runtimeErr)
	}
	return nil
}

func (e *Engine) Start(
	ctx context.Context,
	workflow WorkflowDefinition,
	input string,
) (Transition, error) {
	return e.call(ctx, "start", workflowValue(workflow), wacogo.ValString(input))
}

func (e *Engine) CompleteNode(
	ctx context.Context,
	workflow WorkflowDefinition,
	snapshot ExecutionSnapshot,
	nodeID string,
	output string,
) (Transition, error) {
	return e.call(
		ctx,
		"complete-node",
		workflowValue(workflow),
		snapshotValue(snapshot),
		wacogo.ValString(nodeID),
		wacogo.ValString(output),
	)
}

func (e *Engine) call(ctx context.Context, name string, arguments ...wacogo.Val) (Transition, error) {
	e.mu.Lock()
	defer e.mu.Unlock()
	if e.instance == nil {
		return Transition{}, fmt.Errorf("flower engine is closed")
	}
	function := e.api.ExportedFunc(name)
	if function == nil {
		return Transition{}, fmt.Errorf("flower component does not export %q", name)
	}
	results, err := function.Call(ctx, arguments...)
	if err != nil {
		return Transition{}, fmt.Errorf("call flower component %q: %w", name, err)
	}
	if len(results) != 1 {
		return Transition{}, fmt.Errorf("flower component %q returned %d values, expected 1", name, len(results))
	}
	result, ok := results[0].(*wacogo.ValResult)
	if !ok {
		return Transition{}, unexpectedValue("engine result", results[0])
	}
	if !result.IsOk() {
		return Transition{}, decodeEngineError(result.Err())
	}
	return decodeTransition(result.Ok())
}

func workflowValue(workflow WorkflowDefinition) wacogo.Val {
	nodes := make([]*wacogo.ValRecord, 0, len(workflow.Nodes))
	for _, node := range workflow.Nodes {
		nodes = append(nodes, wacogo.NewValRecord(
			wacogo.Field{Name: "id", Val: wacogo.ValString(node.ID)},
			wacogo.Field{Name: "kind", Val: wacogo.NewValEnum(uint32(node.Kind))},
		))
	}
	edges := make([]*wacogo.ValRecord, 0, len(workflow.Edges))
	for _, edge := range workflow.Edges {
		edges = append(edges, wacogo.NewValRecord(
			wacogo.Field{Name: "id", Val: wacogo.ValString(edge.ID)},
			wacogo.Field{Name: "source", Val: wacogo.ValString(edge.Source)},
			wacogo.Field{Name: "target", Val: wacogo.ValString(edge.Target)},
		))
	}
	return wacogo.NewValRecord(
		wacogo.Field{Name: "id", Val: wacogo.ValString(workflow.ID)},
		wacogo.Field{Name: "nodes", Val: wacogo.NewValListOf(nodes...)},
		wacogo.Field{Name: "edges", Val: wacogo.NewValListOf(edges...)},
	)
}

func snapshotValue(snapshot ExecutionSnapshot) wacogo.Val {
	pendingNodeID := wacogo.ValOptionNone()
	if snapshot.PendingNodeID != nil {
		pendingNodeID = wacogo.ValOptionSome(wacogo.ValString(*snapshot.PendingNodeID))
	}
	completedNodeIDs := make([]wacogo.ValString, 0, len(snapshot.CompletedNodeIDs))
	for _, nodeID := range snapshot.CompletedNodeIDs {
		completedNodeIDs = append(completedNodeIDs, wacogo.ValString(nodeID))
	}
	return wacogo.NewValRecord(
		wacogo.Field{Name: "workflow-id", Val: wacogo.ValString(snapshot.WorkflowID)},
		wacogo.Field{Name: "status", Val: wacogo.NewValEnum(uint32(snapshot.Status))},
		wacogo.Field{Name: "pending-node-id", Val: pendingNodeID},
		wacogo.Field{Name: "current-value", Val: wacogo.ValString(snapshot.CurrentValue)},
		wacogo.Field{
			Name: "completed-node-ids",
			Val:  wacogo.NewValListOf(completedNodeIDs...),
		},
	)
}

func decodeTransition(value wacogo.Val) (Transition, error) {
	record, ok := value.(*wacogo.ValRecord)
	if !ok {
		return Transition{}, unexpectedValue("transition", value)
	}
	snapshot, err := decodeSnapshot(record.Field("snapshot"))
	if err != nil {
		return Transition{}, err
	}
	effectValues, ok := record.Field("effects").(*wacogo.ValList)
	if !ok {
		return Transition{}, unexpectedValue("transition.effects", record.Field("effects"))
	}
	effects := make([]ExecuteNodeEffect, 0, effectValues.Len())
	for index := range effectValues.Len() {
		effect, err := decodeEffect(effectValues.Get(index))
		if err != nil {
			return Transition{}, err
		}
		effects = append(effects, effect)
	}
	return Transition{Snapshot: snapshot, Effects: effects}, nil
}

func decodeSnapshot(value wacogo.Val) (ExecutionSnapshot, error) {
	record, ok := value.(*wacogo.ValRecord)
	if !ok {
		return ExecutionSnapshot{}, unexpectedValue("execution snapshot", value)
	}
	workflowID, err := stringField(record, "workflow-id")
	if err != nil {
		return ExecutionSnapshot{}, err
	}
	statusValue, ok := record.Field("status").(*wacogo.ValEnum)
	if !ok {
		return ExecutionSnapshot{}, unexpectedValue("execution-snapshot.status", record.Field("status"))
	}
	pendingValue, ok := record.Field("pending-node-id").(*wacogo.ValOption)
	if !ok {
		return ExecutionSnapshot{}, unexpectedValue("execution-snapshot.pending-node-id", record.Field("pending-node-id"))
	}
	var pendingNodeID *string
	if !pendingValue.IsNone() {
		value, ok := pendingValue.Val().(wacogo.ValString)
		if !ok {
			return ExecutionSnapshot{}, unexpectedValue("execution-snapshot.pending-node-id.some", pendingValue.Val())
		}
		stringValue := string(value)
		pendingNodeID = &stringValue
	}
	currentValue, err := stringField(record, "current-value")
	if err != nil {
		return ExecutionSnapshot{}, err
	}
	completedValues, ok := record.Field("completed-node-ids").(*wacogo.ValList)
	if !ok {
		return ExecutionSnapshot{}, unexpectedValue("execution-snapshot.completed-node-ids", record.Field("completed-node-ids"))
	}
	completedNodeIDs := make([]string, 0, completedValues.Len())
	for index := range completedValues.Len() {
		value, ok := completedValues.Get(index).(wacogo.ValString)
		if !ok {
			return ExecutionSnapshot{}, unexpectedValue("execution-snapshot.completed-node-ids item", completedValues.Get(index))
		}
		completedNodeIDs = append(completedNodeIDs, string(value))
	}
	return ExecutionSnapshot{
		WorkflowID:       workflowID,
		Status:           ExecutionStatus(statusValue.Discriminant()),
		PendingNodeID:    pendingNodeID,
		CurrentValue:     currentValue,
		CompletedNodeIDs: completedNodeIDs,
	}, nil
}

func decodeEffect(value wacogo.Val) (ExecuteNodeEffect, error) {
	record, ok := value.(*wacogo.ValRecord)
	if !ok {
		return ExecuteNodeEffect{}, unexpectedValue("execute-node effect", value)
	}
	nodeID, err := stringField(record, "node-id")
	if err != nil {
		return ExecuteNodeEffect{}, err
	}
	input, err := stringField(record, "input")
	if err != nil {
		return ExecuteNodeEffect{}, err
	}
	return ExecuteNodeEffect{NodeID: nodeID, Input: input}, nil
}

func decodeEngineError(value wacogo.Val) error {
	record, ok := value.(*wacogo.ValRecord)
	if !ok {
		return unexpectedValue("engine error", value)
	}
	code, err := stringField(record, "code")
	if err != nil {
		return err
	}
	message, err := stringField(record, "message")
	if err != nil {
		return err
	}
	return &EngineError{Code: code, Message: message}
}

func stringField(record *wacogo.ValRecord, name string) (string, error) {
	value, ok := record.Field(name).(wacogo.ValString)
	if !ok {
		return "", unexpectedValue(name, record.Field(name))
	}
	return string(value), nil
}

func unexpectedValue(name string, value wacogo.Val) error {
	return fmt.Errorf("flower component returned invalid %s value %T", name, value)
}
