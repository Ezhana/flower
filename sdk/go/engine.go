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
	abiDefinition, err := definitionToABI(definition)
	if err != nil {
		return ExecutableWorkflowPlan{}, nil, err
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

func definitionToABI(value WorkflowDefinition) (abi.WorkflowDefinition, error) {
	nodes := make([]abi.NodeDefinition, len(value.Nodes))
	for index, node := range value.Nodes {
		kind, err := nodeKindToABI(node.Kind)
		if err != nil {
			return abi.WorkflowDefinition{}, fmt.Errorf("convert definition.nodes[%d] %q: %w", index, node.ID, err)
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
	if err := validateIdentifier("plan workflow id", value.WorkflowID); err != nil {
		return abi.ExecutableWorkflowPlan{}, err
	}
	if err := validateFingerprint("plan fingerprint", value.Fingerprint); err != nil {
		return abi.ExecutableWorkflowPlan{}, err
	}
	nodes := make([]abi.PlanNode, len(value.Nodes))
	for index, node := range value.Nodes {
		if err := validateIdentifier(fmt.Sprintf("plan.nodes[%d] id", index), node.ID); err != nil {
			return abi.ExecutableWorkflowPlan{}, err
		}
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

func payloadToABI(value Payload) (abi.Payload, error) {
	return abi.Payload{MediaType: value.MediaType, Bytes: append([]byte(nil), value.Bytes...)}, nil
}
func payloadFromABI(value abi.Payload) Payload {
	return Payload{MediaType: value.MediaType, Bytes: append([]byte(nil), value.Bytes...)}
}

func planReferenceToABI(value PlanReference) (abi.PlanReference, error) {
	if err := validateIdentifier("plan reference workflow id", value.WorkflowID); err != nil {
		return abi.PlanReference{}, err
	}
	if err := validateFingerprint("plan reference fingerprint", value.Fingerprint); err != nil {
		return abi.PlanReference{}, err
	}
	return abi.PlanReference{
		SpecificationVersion: abi.SpecificationVersion{
			Major: value.SpecificationVersion.Major,
			Minor: value.SpecificationVersion.Minor,
		},
		WorkflowID:  value.WorkflowID,
		Fingerprint: value.Fingerprint,
	}, nil
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
	if err := validateIdentifier("snapshot execution id", value.ExecutionID); err != nil {
		return abi.ExecutionSnapshot{}, err
	}
	planReference, err := planReferenceToABI(value.PlanReference)
	if err != nil {
		return abi.ExecutionSnapshot{}, fmt.Errorf("convert snapshot: %w", err)
	}
	var state abi.ExecutionState
	switch current := value.State.(type) {
	case AwaitingNode:
		if err := validateIdentifier("snapshot awaiting node id", current.NodeID); err != nil {
			return abi.ExecutionSnapshot{}, err
		}
		if err := validateIdentifier("snapshot awaiting effect id", current.EffectID); err != nil {
			return abi.ExecutionSnapshot{}, err
		}
		input, err := payloadToABI(current.Input)
		if err != nil {
			return abi.ExecutionSnapshot{}, fmt.Errorf("convert snapshot awaiting input: %w", err)
		}
		state = abi.ExecutionStateAwaitingNode{Value: abi.AwaitingNode{NodeID: current.NodeID, EffectID: current.EffectID, Input: input}}
	case Completed:
		output, err := payloadToABI(current.Output)
		if err != nil {
			return abi.ExecutionSnapshot{}, fmt.Errorf("convert snapshot completed output: %w", err)
		}
		state = abi.ExecutionStateCompleted{Value: output}
	default:
		return abi.ExecutionSnapshot{}, fmt.Errorf("unsupported execution state %T", value.State)
	}
	return abi.ExecutionSnapshot{
		ExecutionID:   value.ExecutionID,
		PlanReference: planReference,
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
		if err := validateIdentifier("execution-started event id", event.EventID); err != nil {
			return nil, err
		}
		if err := validateIdentifier("execution-started execution id", event.ExecutionID); err != nil {
			return nil, err
		}
		planReference, err := planReferenceToABI(event.PlanReference)
		if err != nil {
			return nil, fmt.Errorf("convert execution-started: %w", err)
		}
		input, err := payloadToABI(event.Input)
		if err != nil {
			return nil, fmt.Errorf("convert execution-started input: %w", err)
		}
		return abi.ExecutionEventExecutionStarted{Value: abi.ExecutionStartedEvent{
			EventID:       event.EventID,
			ExecutionID:   event.ExecutionID,
			PlanReference: planReference,
			Input:         input,
		}}, nil
	case NodeCompleted:
		identifiers := []struct {
			field string
			value string
		}{
			{field: "node-completed event id", value: event.EventID},
			{field: "node-completed execution id", value: event.ExecutionID},
			{field: "node-completed effect id", value: event.EffectID},
			{field: "node-completed node id", value: event.NodeID},
		}
		for _, identifier := range identifiers {
			if err := validateIdentifier(identifier.field, identifier.value); err != nil {
				return nil, err
			}
		}
		output, err := payloadToABI(event.Output)
		if err != nil {
			return nil, fmt.Errorf("convert node-completed output: %w", err)
		}
		return abi.ExecutionEventNodeCompleted{Value: abi.NodeCompletedEvent{EventID: event.EventID, ExecutionID: event.ExecutionID, ExpectedRevision: event.ExpectedRevision, EffectID: event.EffectID, NodeID: event.NodeID, Output: output}}, nil
	default:
		return nil, fmt.Errorf("unsupported execution event %T", value)
	}
}

func validateIdentifier(field, value string) error {
	if value == "" {
		return fmt.Errorf("%s must not be empty", field)
	}
	for _, character := range value {
		if (character >= 'a' && character <= 'z') ||
			(character >= 'A' && character <= 'Z') ||
			(character >= '0' && character <= '9') ||
			character == '.' || character == '_' || character == '-' {
			continue
		}
		return fmt.Errorf("%s %q contains unsupported characters", field, value)
	}
	return nil
}

func validateFingerprint(field, value string) error {
	if len(value) != 64 {
		return fmt.Errorf("%s must contain exactly 64 hexadecimal characters", field)
	}
	for _, character := range value {
		if (character >= 'a' && character <= 'f') || (character >= '0' && character <= '9') {
			continue
		}
		return fmt.Errorf("%s must use lowercase hexadecimal encoding", field)
	}
	return nil
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
