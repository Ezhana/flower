package flower

import (
	"context"
	"fmt"
	"io"
	"sync"

	"flower.dev/sdk/go/internal/componentabi"
	abi "flower.dev/sdk/go/internal/componentabi/flower/engine/workflowengine"
)

type Engine interface {
	Compile(context.Context, WorkflowDefinition) (ExecutableWorkflowPlan, []Diagnostic, error)
	Transition(context.Context, ExecutableWorkflowPlan, *ExecutionSnapshot, ExecutionEvent) (Transition, error)
	Close(context.Context) error
}

type componentEngine struct {
	client *componentabi.Client
	mu     sync.Mutex
	closed bool
}

func LoadEngine(ctx context.Context, component io.Reader) (Engine, error) {
	client, err := componentabi.Load(ctx, component)
	if err != nil {
		return nil, err
	}
	return &componentEngine{client: client}, nil
}

func (e *componentEngine) Close(ctx context.Context) error {
	e.mu.Lock()
	defer e.mu.Unlock()
	if e.closed {
		return nil
	}
	err := e.client.Close(ctx)
	if err == nil {
		e.closed = true
	}
	return err
}

func (e *componentEngine) Compile(ctx context.Context, definition WorkflowDefinition) (ExecutableWorkflowPlan, []Diagnostic, error) {
	e.mu.Lock()
	defer e.mu.Unlock()
	if e.closed {
		return ExecutableWorkflowPlan{}, nil, fmt.Errorf("flower engine is closed")
	}
	abiDefinition, diagnostics := definitionToABI(definition)
	if len(diagnostics) != 0 {
		return ExecutableWorkflowPlan{}, diagnostics, nil
	}
	result, err := e.client.Compile(ctx, abiDefinition)
	if err != nil {
		return ExecutableWorkflowPlan{}, nil, fmt.Errorf("compile workflow component call: %w", err)
	}
	switch value := result.(type) {
	case abi.ResultExecutableWorkflowPlanListDiagnosticOk:
		plan, err := planFromABI(value.Value)
		return plan, nil, err
	case abi.ResultExecutableWorkflowPlanListDiagnosticErr:
		diagnostics := make([]Diagnostic, len(value.Value))
		for index, diagnostic := range value.Value {
			var subject *string
			if diagnostic.Subject.IsSome {
				copy := diagnostic.Subject.Value
				subject = &copy
			}
			diagnostics[index] = Diagnostic{Code: diagnostic.Code, Message: diagnostic.Message, Subject: subject}
		}
		return ExecutableWorkflowPlan{}, diagnostics, nil
	default:
		return ExecutableWorkflowPlan{}, nil, fmt.Errorf("component returned unknown compile result %T", result)
	}
}

func (e *componentEngine) Transition(ctx context.Context, plan ExecutableWorkflowPlan, snapshot *ExecutionSnapshot, event ExecutionEvent) (Transition, error) {
	e.mu.Lock()
	defer e.mu.Unlock()
	if e.closed {
		return Transition{}, fmt.Errorf("flower engine is closed")
	}
	abiSnapshot := abi.NoneExecutionSnapshot()
	if snapshot != nil {
		converted, err := snapshotToABI(*snapshot)
		if err != nil {
			return Transition{}, err
		}
		abiSnapshot = abi.SomeExecutionSnapshot(converted)
	}
	abiEvent, err := eventToABI(event)
	if err != nil {
		return Transition{}, err
	}
	abiPlan, err := planToABI(plan)
	if err != nil {
		return Transition{}, err
	}
	result, err := e.client.Transition(ctx, abiPlan, abiSnapshot, abiEvent)
	if err != nil {
		return Transition{}, fmt.Errorf("transition component call: %w", err)
	}
	switch value := result.(type) {
	case abi.ResultTransitionResultEngineErrorOk:
		return transitionFromABI(value.Value)
	case abi.ResultTransitionResultEngineErrorErr:
		return Transition{}, &EngineError{Code: value.Value.Code, Message: value.Value.Message}
	default:
		return Transition{}, fmt.Errorf("component returned unknown transition result %T", result)
	}
}

func definitionToABI(value WorkflowDefinition) (abi.WorkflowDefinition, []Diagnostic) {
	nodes := make([]abi.NodeDefinition, len(value.Nodes))
	for index, node := range value.Nodes {
		kind, err := nodeKindToABI(node.Kind)
		if err != nil {
			subject := node.ID
			return abi.WorkflowDefinition{}, []Diagnostic{{
				Code:    "invalid-node-kind",
				Message: fmt.Sprintf("node %q: %v", node.ID, err),
				Subject: &subject,
			}}
		}
		nodes[index] = abi.NodeDefinition{ID: node.ID, Kind: kind}
	}
	edges := make([]abi.EdgeDefinition, len(value.Edges))
	for index, edge := range value.Edges {
		edges[index] = abi.EdgeDefinition{ID: edge.ID, Source: edge.Source, Target: edge.Target}
	}
	return abi.WorkflowDefinition{ID: value.ID, Nodes: nodes, Edges: edges}, nil
}

func nodeKindToABI(value NodeKind) (abi.NodeKind, error) {
	switch value {
	case StartNode:
		return abi.NodeKindStart, nil
	case ActivityNode:
		return abi.NodeKindActivity, nil
	case FinishNode:
		return abi.NodeKindFinish, nil
	default:
		return 0, fmt.Errorf("unsupported node kind %q", value)
	}
}
func nodeKindFromABI(value abi.NodeKind) (NodeKind, error) {
	switch value {
	case abi.NodeKindStart:
		return StartNode, nil
	case abi.NodeKindActivity:
		return ActivityNode, nil
	case abi.NodeKindFinish:
		return FinishNode, nil
	default:
		return "", fmt.Errorf("component returned unknown node-kind discriminant %d", value)
	}
}

func planToABI(value ExecutableWorkflowPlan) (abi.ExecutableWorkflowPlan, error) {
	nodes := make([]abi.PlanNode, len(value.Nodes))
	for index, node := range value.Nodes {
		kind, err := nodeKindToABI(node.Kind)
		if err != nil {
			return abi.ExecutableWorkflowPlan{}, fmt.Errorf(
				"invalid executable plan node %q: %w",
				node.ID,
				err,
			)
		}
		nodes[index] = abi.PlanNode{ID: node.ID, Kind: kind}
	}
	return abi.ExecutableWorkflowPlan{SpecificationVersion: abi.SpecificationVersion{Major: value.SpecificationVersion.Major, Minor: value.SpecificationVersion.Minor}, WorkflowID: value.WorkflowID, Fingerprint: value.Fingerprint, Nodes: nodes}, nil
}

func planFromABI(value abi.ExecutableWorkflowPlan) (ExecutableWorkflowPlan, error) {
	nodes := make([]PlanNode, len(value.Nodes))
	for index, node := range value.Nodes {
		kind, err := nodeKindFromABI(node.Kind)
		if err != nil {
			return ExecutableWorkflowPlan{}, fmt.Errorf("decode plan.nodes[%d]: %w", index, err)
		}
		nodes[index] = PlanNode{ID: node.ID, Kind: kind}
	}
	return ExecutableWorkflowPlan{SpecificationVersion: SpecificationVersion{Major: value.SpecificationVersion.Major, Minor: value.SpecificationVersion.Minor}, WorkflowID: value.WorkflowID, Fingerprint: value.Fingerprint, Nodes: nodes}, nil
}

func payloadToABI(value Payload) abi.Payload {
	return abi.Payload{MediaType: value.MediaType, Bytes: append([]byte(nil), value.Bytes...)}
}
func payloadFromABI(value abi.Payload) Payload {
	return Payload{MediaType: value.MediaType, Bytes: append([]byte(nil), value.Bytes...)}
}

func planReferenceToABI(value PlanReference) abi.PlanReference {
	return abi.PlanReference{
		SpecificationVersion: abi.SpecificationVersion{
			Major: value.SpecificationVersion.Major,
			Minor: value.SpecificationVersion.Minor,
		},
		WorkflowID:  value.WorkflowID,
		Fingerprint: value.Fingerprint,
	}
}

func planReferenceFromABI(value abi.PlanReference) PlanReference {
	return PlanReference{
		SpecificationVersion: SpecificationVersion{
			Major: value.SpecificationVersion.Major,
			Minor: value.SpecificationVersion.Minor,
		},
		WorkflowID:  value.WorkflowID,
		Fingerprint: value.Fingerprint,
	}
}

func snapshotToABI(value ExecutionSnapshot) (abi.ExecutionSnapshot, error) {
	var state abi.ExecutionState
	switch current := value.State.(type) {
	case AwaitingNode:
		state = abi.ExecutionStateAwaitingNode{Value: abi.AwaitingNode{NodeID: current.NodeID, EffectID: current.EffectID, Input: payloadToABI(current.Input)}}
	case Completed:
		state = abi.ExecutionStateCompleted{Value: payloadToABI(current.Output)}
	default:
		return abi.ExecutionSnapshot{}, fmt.Errorf("unsupported execution state %T", value.State)
	}
	return abi.ExecutionSnapshot{
		ExecutionID:   value.ExecutionID,
		PlanReference: planReferenceToABI(value.PlanReference),
		Revision:      value.Revision,
		State:         state,
	}, nil
}

func snapshotFromABI(value abi.ExecutionSnapshot) (ExecutionSnapshot, error) {
	var state ExecutionState
	switch current := value.State.(type) {
	case abi.ExecutionStateAwaitingNode:
		state = AwaitingNode{NodeID: current.Value.NodeID, EffectID: current.Value.EffectID, Input: payloadFromABI(current.Value.Input)}
	case abi.ExecutionStateCompleted:
		state = Completed{Output: payloadFromABI(current.Value)}
	default:
		return ExecutionSnapshot{}, fmt.Errorf("component returned unknown execution-state discriminant %T", value.State)
	}
	return ExecutionSnapshot{
		ExecutionID:   value.ExecutionID,
		PlanReference: planReferenceFromABI(value.PlanReference),
		Revision:      value.Revision,
		State:         state,
	}, nil
}

func eventToABI(value ExecutionEvent) (abi.ExecutionEvent, error) {
	switch event := value.(type) {
	case ExecutionStarted:
		return abi.ExecutionEventExecutionStarted{Value: abi.ExecutionStartedEvent{
			EventID:       event.EventID,
			ExecutionID:   event.ExecutionID,
			PlanReference: planReferenceToABI(event.PlanReference),
			Input:         payloadToABI(event.Input),
		}}, nil
	case NodeCompleted:
		return abi.ExecutionEventNodeCompleted{Value: abi.NodeCompletedEvent{EventID: event.EventID, ExecutionID: event.ExecutionID, ExpectedRevision: event.ExpectedRevision, EffectID: event.EffectID, NodeID: event.NodeID, Output: payloadToABI(event.Output)}}, nil
	default:
		return nil, fmt.Errorf("unsupported execution event %T", value)
	}
}

func transitionFromABI(value abi.TransitionResult) (Transition, error) {
	snapshot, err := snapshotFromABI(value.Snapshot)
	if err != nil {
		return Transition{}, fmt.Errorf("decode transition snapshot: %w", err)
	}
	effects := make([]ExecutionEffect, len(value.Effects))
	for index, effect := range value.Effects {
		switch current := effect.(type) {
		case abi.ExecutionEffectExecuteNode:
			effects[index] = ExecuteNode{EffectID: current.Value.EffectID, ExecutionID: current.Value.ExecutionID, NodeID: current.Value.NodeID, Input: payloadFromABI(current.Value.Input)}
		default:
			return Transition{}, fmt.Errorf("component returned unknown execution-effect discriminant at %d: %T", index, effect)
		}
	}
	return Transition{Snapshot: snapshot, Effects: effects}, nil
}
