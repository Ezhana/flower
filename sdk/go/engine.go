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
		retryPolicy := abi.NoneRetryPolicy()
		if node.RetryPolicy != nil {
			converted, err := retryPolicyToABI(*node.RetryPolicy)
			if err != nil {
				return abi.WorkflowDefinition{}, fmt.Errorf("convert definition.nodes[%d] %q retry policy: %w", index, node.ID, err)
			}
			retryPolicy = abi.SomeRetryPolicy(converted)
		}
		nodes[index] = abi.NodeDefinition{ID: node.ID, Kind: kind, RetryPolicy: retryPolicy}
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

func retryPolicyToABI(value RetryPolicy) (abi.RetryPolicy, error) {
	if value.MaxAttempts == 0 {
		return abi.RetryPolicy{}, fmt.Errorf("max attempts must be non-zero")
	}
	for index, code := range value.RetryableFailureCodes {
		if err := validateIdentifier(fmt.Sprintf("retryable failure code %d", index), code); err != nil {
			return abi.RetryPolicy{}, err
		}
	}
	var backoff abi.BackoffPolicy
	switch value.Backoff.Type {
	case NoBackoff:
		if value.Backoff.DelayMs != 0 || value.Backoff.InitialDelayMs != 0 || value.Backoff.Multiplier != 0 || value.Backoff.MaximumDelayMs != 0 {
			return abi.RetryPolicy{}, fmt.Errorf("none backoff cannot contain numeric parameters")
		}
		backoff = abi.BackoffPolicyNone{}
	case FixedBackoff:
		if value.Backoff.InitialDelayMs != 0 || value.Backoff.Multiplier != 0 || value.Backoff.MaximumDelayMs != 0 {
			return abi.RetryPolicy{}, fmt.Errorf("fixed backoff contains exponential parameters")
		}
		backoff = abi.BackoffPolicyFixed{Value: abi.FixedBackoff{DelayMs: value.Backoff.DelayMs}}
	case ExponentialBackoff:
		if value.Backoff.DelayMs != 0 || value.Backoff.Multiplier == 0 || value.Backoff.MaximumDelayMs < value.Backoff.InitialDelayMs {
			return abi.RetryPolicy{}, fmt.Errorf("exponential backoff violates its numeric invariants")
		}
		backoff = abi.BackoffPolicyExponential{Value: abi.ExponentialBackoff{
			InitialDelayMs: value.Backoff.InitialDelayMs,
			Multiplier:     value.Backoff.Multiplier,
			MaximumDelayMs: value.Backoff.MaximumDelayMs,
		}}
	default:
		return abi.RetryPolicy{}, fmt.Errorf("unsupported backoff kind %q", value.Backoff.Type)
	}
	return abi.RetryPolicy{
		MaxAttempts:           value.MaxAttempts,
		RetryableFailureCodes: append([]string(nil), value.RetryableFailureCodes...),
		Backoff:               backoff,
	}, nil
}

func retryPolicyFromABI(value abi.RetryPolicy) (RetryPolicy, error) {
	var backoff BackoffPolicy
	switch current := value.Backoff.(type) {
	case abi.BackoffPolicyNone:
		backoff = BackoffPolicy{Type: NoBackoff}
	case abi.BackoffPolicyFixed:
		backoff = BackoffPolicy{Type: FixedBackoff, DelayMs: current.Value.DelayMs}
	case abi.BackoffPolicyExponential:
		backoff = BackoffPolicy{
			Type: ExponentialBackoff, InitialDelayMs: current.Value.InitialDelayMs,
			Multiplier: current.Value.Multiplier, MaximumDelayMs: current.Value.MaximumDelayMs,
		}
	default:
		return RetryPolicy{}, fmt.Errorf("component returned unknown backoff discriminant %T", value.Backoff)
	}
	return RetryPolicy{
		MaxAttempts:           value.MaxAttempts,
		RetryableFailureCodes: append([]string{}, value.RetryableFailureCodes...),
		Backoff:               backoff,
	}, nil
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
		retryPolicy := abi.NoneRetryPolicy()
		if node.RetryPolicy != nil {
			converted, err := retryPolicyToABI(*node.RetryPolicy)
			if err != nil {
				return abi.ExecutableWorkflowPlan{}, fmt.Errorf("invalid executable plan node %q retry policy: %w", node.ID, err)
			}
			retryPolicy = abi.SomeRetryPolicy(converted)
		}
		nodes[index] = abi.PlanNode{ID: node.ID, Kind: kind, RetryPolicy: retryPolicy}
	}
	return abi.ExecutableWorkflowPlan{WorkflowID: value.WorkflowID, Fingerprint: value.Fingerprint, Nodes: nodes}, nil
}

func planFromABI(value abi.ExecutableWorkflowPlan) (ExecutableWorkflowPlan, error) {
	nodes := make([]PlanNode, len(value.Nodes))
	for index, node := range value.Nodes {
		kind, err := nodeKindFromABI(node.Kind)
		if err != nil {
			return ExecutableWorkflowPlan{}, fmt.Errorf("decode plan.nodes[%d]: %w", index, err)
		}
		var retryPolicy *RetryPolicy
		if node.RetryPolicy.IsSome {
			converted, err := retryPolicyFromABI(node.RetryPolicy.Value)
			if err != nil {
				return ExecutableWorkflowPlan{}, fmt.Errorf("decode plan.nodes[%d] retry policy: %w", index, err)
			}
			retryPolicy = &converted
		}
		nodes[index] = PlanNode{ID: node.ID, Kind: kind, RetryPolicy: retryPolicy}
	}
	return ExecutableWorkflowPlan{WorkflowID: value.WorkflowID, Fingerprint: value.Fingerprint, Nodes: nodes}, nil
}

func payloadToABI(value Payload) (abi.Payload, error) {
	return abi.Payload{MediaType: value.MediaType, Bytes: append([]byte(nil), value.Bytes...)}, nil
}
func payloadFromABI(value abi.Payload) Payload {
	return Payload{MediaType: value.MediaType, Bytes: append([]byte(nil), value.Bytes...)}
}

func activationToABI(value NodeActivation) (abi.NodeActivation, error) {
	if err := validateIdentifier("activation id", value.ActivationID); err != nil {
		return abi.NodeActivation{}, err
	}
	if err := validateIdentifier("activation node id", value.NodeID); err != nil {
		return abi.NodeActivation{}, err
	}
	input, err := payloadToABI(value.Input)
	if err != nil {
		return abi.NodeActivation{}, fmt.Errorf("convert activation input: %w", err)
	}
	return abi.NodeActivation{
		ActivationID:       value.ActivationID,
		ActivationRevision: value.ActivationRevision,
		NodeID:             value.NodeID,
		Input:              input,
	}, nil
}

func activationFromABI(value abi.NodeActivation) NodeActivation {
	return NodeActivation{
		ActivationID:       value.ActivationID,
		ActivationRevision: value.ActivationRevision,
		NodeID:             value.NodeID,
		Input:              payloadFromABI(value.Input),
	}
}

func attemptToABI(value NodeAttempt) (abi.NodeAttempt, error) {
	if err := validateIdentifier("attempt id", value.AttemptID); err != nil {
		return abi.NodeAttempt{}, err
	}
	if value.AttemptNumber == 0 {
		return abi.NodeAttempt{}, fmt.Errorf("attempt number must be non-zero")
	}
	if err := validateIdentifier("attempt effect id", value.EffectID); err != nil {
		return abi.NodeAttempt{}, err
	}
	return abi.NodeAttempt{
		AttemptID:     value.AttemptID,
		AttemptNumber: value.AttemptNumber,
		EffectID:      value.EffectID,
	}, nil
}

func attemptFromABI(value abi.NodeAttempt) NodeAttempt {
	return NodeAttempt{
		AttemptID:     value.AttemptID,
		AttemptNumber: value.AttemptNumber,
		EffectID:      value.EffectID,
	}
}

func failureToABI(value AttemptFailure) (abi.AttemptFailure, error) {
	if err := validateIdentifier("failure code", value.Code); err != nil {
		return abi.AttemptFailure{}, err
	}
	details := abi.NonePayload()
	if value.Details != nil {
		converted, err := payloadToABI(*value.Details)
		if err != nil {
			return abi.AttemptFailure{}, fmt.Errorf("convert failure details: %w", err)
		}
		details = abi.SomePayload(converted)
	}
	return abi.AttemptFailure{Code: value.Code, Details: details}, nil
}

func failureFromABI(value abi.AttemptFailure) AttemptFailure {
	var details *Payload
	if value.Details.IsSome {
		converted := payloadFromABI(value.Details.Value)
		details = &converted
	}
	return AttemptFailure{Code: value.Code, Details: details}
}

func retryTimerToABI(value RetryTimer) (abi.RetryTimer, error) {
	identifiers := []struct{ field, value string }{
		{field: "retry timer id", value: value.TimerID},
		{field: "retry timer effect id", value: value.EffectID},
		{field: "retry timer failed attempt id", value: value.FailedAttemptID},
	}
	for _, identifier := range identifiers {
		if err := validateIdentifier(identifier.field, identifier.value); err != nil {
			return abi.RetryTimer{}, err
		}
	}
	if value.NextAttemptNumber == 0 {
		return abi.RetryTimer{}, fmt.Errorf("retry timer next attempt number must be non-zero")
	}
	return abi.RetryTimer{
		TimerID: value.TimerID, EffectID: value.EffectID, FailedAttemptID: value.FailedAttemptID,
		NextAttemptNumber: value.NextAttemptNumber, DelayMs: value.DelayMs,
	}, nil
}

func retryTimerFromABI(value abi.RetryTimer) RetryTimer {
	return RetryTimer{
		TimerID: value.TimerID, EffectID: value.EffectID, FailedAttemptID: value.FailedAttemptID,
		NextAttemptNumber: value.NextAttemptNumber, DelayMs: value.DelayMs,
	}
}

func planReferenceToABI(value PlanReference) (abi.PlanReference, error) {
	if err := validateIdentifier("plan reference workflow id", value.WorkflowID); err != nil {
		return abi.PlanReference{}, err
	}
	if err := validateFingerprint("plan reference fingerprint", value.Fingerprint); err != nil {
		return abi.PlanReference{}, err
	}
	return abi.PlanReference{
		WorkflowID: value.WorkflowID, Fingerprint: value.Fingerprint,
	}, nil
}

func planReferenceFromABI(value abi.PlanReference) PlanReference {
	return PlanReference{
		WorkflowID: value.WorkflowID, Fingerprint: value.Fingerprint,
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
	case AwaitingAttempt:
		activation, err := activationToABI(current.Activation)
		if err != nil {
			return abi.ExecutionSnapshot{}, fmt.Errorf("convert awaiting activation: %w", err)
		}
		attempt, err := attemptToABI(current.Attempt)
		if err != nil {
			return abi.ExecutionSnapshot{}, fmt.Errorf("convert awaiting attempt: %w", err)
		}
		state = abi.ExecutionStateAwaitingAttempt{Value: abi.AwaitingAttempt{Activation: activation, Attempt: attempt}}
	case WaitingForRetry:
		activation, err := activationToABI(current.Activation)
		if err != nil {
			return abi.ExecutionSnapshot{}, fmt.Errorf("convert retry activation: %w", err)
		}
		attempt, err := attemptToABI(current.Attempt)
		if err != nil {
			return abi.ExecutionSnapshot{}, fmt.Errorf("convert retry attempt: %w", err)
		}
		failure, err := failureToABI(current.Failure)
		if err != nil {
			return abi.ExecutionSnapshot{}, fmt.Errorf("convert retry failure: %w", err)
		}
		timer, err := retryTimerToABI(current.Timer)
		if err != nil {
			return abi.ExecutionSnapshot{}, fmt.Errorf("convert retry timer: %w", err)
		}
		state = abi.ExecutionStateWaitingForRetry{Value: abi.WaitingForRetry{
			Activation: activation, Attempt: attempt, Failure: failure, Timer: timer,
		}}
	case Completed:
		output, err := payloadToABI(current.Output)
		if err != nil {
			return abi.ExecutionSnapshot{}, fmt.Errorf("convert snapshot completed output: %w", err)
		}
		state = abi.ExecutionStateCompleted{Value: output}
	case Failed:
		activation, err := activationToABI(current.Activation)
		if err != nil {
			return abi.ExecutionSnapshot{}, fmt.Errorf("convert failed activation: %w", err)
		}
		attempt, err := attemptToABI(current.Attempt)
		if err != nil {
			return abi.ExecutionSnapshot{}, fmt.Errorf("convert failed attempt: %w", err)
		}
		failure, err := failureToABI(current.Failure)
		if err != nil {
			return abi.ExecutionSnapshot{}, fmt.Errorf("convert failed failure: %w", err)
		}
		state = abi.ExecutionStateFailed{Value: abi.FailedExecution{Activation: activation, Attempt: attempt, Failure: failure}}
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
	case abi.ExecutionStateAwaitingAttempt:
		state = AwaitingAttempt{Activation: activationFromABI(current.Value.Activation), Attempt: attemptFromABI(current.Value.Attempt)}
	case abi.ExecutionStateWaitingForRetry:
		state = WaitingForRetry{
			Activation: activationFromABI(current.Value.Activation),
			Attempt:    attemptFromABI(current.Value.Attempt),
			Failure:    failureFromABI(current.Value.Failure),
			Timer:      retryTimerFromABI(current.Value.Timer),
		}
	case abi.ExecutionStateCompleted:
		state = Completed{Output: payloadFromABI(current.Value)}
	case abi.ExecutionStateFailed:
		state = Failed{
			Activation: activationFromABI(current.Value.Activation),
			Attempt:    attemptFromABI(current.Value.Attempt),
			Failure:    failureFromABI(current.Value.Failure),
		}
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
			ExecutionID:   event.ExecutionID,
			PlanReference: planReference,
			Input:         input,
		}}, nil
	case NodeAttemptSucceeded:
		identifiers := []struct {
			field string
			value string
		}{
			{field: "node-attempt-succeeded execution id", value: event.ExecutionID},
			{field: "node-attempt-succeeded activation id", value: event.ActivationID},
			{field: "node-attempt-succeeded attempt id", value: event.AttemptID},
			{field: "node-attempt-succeeded effect id", value: event.EffectID},
			{field: "node-attempt-succeeded node id", value: event.NodeID},
		}
		for _, identifier := range identifiers {
			if err := validateIdentifier(identifier.field, identifier.value); err != nil {
				return nil, err
			}
		}
		if event.AttemptNumber == 0 {
			return nil, fmt.Errorf("node-attempt-succeeded attempt number must be non-zero")
		}
		output, err := payloadToABI(event.Output)
		if err != nil {
			return nil, fmt.Errorf("convert node-attempt-succeeded output: %w", err)
		}
		return abi.ExecutionEventNodeAttemptSucceeded{Value: abi.NodeAttemptSucceededEvent{
			ExecutionID: event.ExecutionID, ExpectedRevision: event.ExpectedRevision,
			ActivationID: event.ActivationID, AttemptID: event.AttemptID, AttemptNumber: event.AttemptNumber,
			EffectID: event.EffectID, NodeID: event.NodeID, Output: output,
		}}, nil
	case NodeAttemptFailed:
		identifiers := []struct{ field, value string }{
			{field: "node-attempt-failed execution id", value: event.ExecutionID},
			{field: "node-attempt-failed activation id", value: event.ActivationID},
			{field: "node-attempt-failed attempt id", value: event.AttemptID},
			{field: "node-attempt-failed effect id", value: event.EffectID},
			{field: "node-attempt-failed node id", value: event.NodeID},
		}
		for _, identifier := range identifiers {
			if err := validateIdentifier(identifier.field, identifier.value); err != nil {
				return nil, err
			}
		}
		if event.AttemptNumber == 0 {
			return nil, fmt.Errorf("node-attempt-failed attempt number must be non-zero")
		}
		failure, err := failureToABI(event.Failure)
		if err != nil {
			return nil, fmt.Errorf("convert node-attempt-failed failure: %w", err)
		}
		return abi.ExecutionEventNodeAttemptFailed{Value: abi.NodeAttemptFailedEvent{
			ExecutionID: event.ExecutionID, ExpectedRevision: event.ExpectedRevision,
			ActivationID: event.ActivationID, AttemptID: event.AttemptID, AttemptNumber: event.AttemptNumber,
			EffectID: event.EffectID, NodeID: event.NodeID, Failure: failure,
		}}, nil
	case TimerFired:
		identifiers := []struct{ field, value string }{
			{field: "timer-fired execution id", value: event.ExecutionID},
			{field: "timer-fired timer id", value: event.TimerID},
			{field: "timer-fired activation id", value: event.ActivationID},
		}
		for _, identifier := range identifiers {
			if err := validateIdentifier(identifier.field, identifier.value); err != nil {
				return nil, err
			}
		}
		if event.NextAttemptNumber == 0 {
			return nil, fmt.Errorf("timer-fired next attempt number must be non-zero")
		}
		return abi.ExecutionEventTimerFired{Value: abi.TimerFiredEvent{
			ExecutionID:      event.ExecutionID,
			ExpectedRevision: event.ExpectedRevision, TimerID: event.TimerID,
			ActivationID: event.ActivationID, NextAttemptNumber: event.NextAttemptNumber,
		}}, nil
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
		case abi.ExecutionEffectExecuteNodeAttempt:
			effects[index] = ExecuteNodeAttempt{
				EffectID: current.Value.EffectID, ActivationID: current.Value.ActivationID,
				AttemptID: current.Value.AttemptID, AttemptNumber: current.Value.AttemptNumber,
				NodeID: current.Value.NodeID, Input: payloadFromABI(current.Value.Input),
			}
		case abi.ExecutionEffectScheduleTimer:
			effects[index] = ScheduleTimer{
				EffectID: current.Value.EffectID, TimerID: current.Value.TimerID,
				ActivationID: current.Value.ActivationID, FailedAttemptID: current.Value.FailedAttemptID,
				NextAttemptNumber: current.Value.NextAttemptNumber, DelayMs: current.Value.DelayMs,
			}
		default:
			return Transition{}, fmt.Errorf("component returned unknown execution-effect discriminant at %d: %T", index, effect)
		}
	}
	return Transition{Snapshot: snapshot, Effects: effects}, nil
}
