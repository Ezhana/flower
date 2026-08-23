package flower

type Payload struct {
	MediaType string `json:"media_type"`
	Bytes     []byte `json:"bytes"`
}

type ExecutionState interface{ executionState() }

type NodeActivation struct {
	ActivationID       string  `json:"activation_id"`
	ActivationRevision uint64  `json:"activation_revision"`
	NodeID             string  `json:"node_id"`
	Input              Payload `json:"input"`
}

type NodeAttempt struct {
	AttemptID     string `json:"attempt_id"`
	AttemptNumber uint32 `json:"attempt_number"`
	EffectID      string `json:"effect_id"`
}

type AttemptFailure struct {
	Code    string   `json:"code"`
	Details *Payload `json:"details"`
}

type RetryTimer struct {
	TimerID           string `json:"timer_id"`
	EffectID          string `json:"effect_id"`
	FailedAttemptID   string `json:"failed_attempt_id"`
	NextAttemptNumber uint32 `json:"next_attempt_number"`
	DelayMs           uint64 `json:"delay_ms"`
}

type AwaitingAttempt struct {
	Activation NodeActivation `json:"activation"`
	Attempt    NodeAttempt    `json:"attempt"`
}

func (AwaitingAttempt) executionState() {}

type WaitingForRetry struct {
	Activation NodeActivation `json:"activation"`
	Attempt    NodeAttempt    `json:"attempt"`
	Failure    AttemptFailure `json:"failure"`
	Timer      RetryTimer     `json:"timer"`
}

func (WaitingForRetry) executionState() {}

type Completed struct {
	Output Payload `json:"output"`
}

func (Completed) executionState() {}

type Failed struct {
	Activation NodeActivation `json:"activation"`
	Attempt    NodeAttempt    `json:"attempt"`
	Failure    AttemptFailure `json:"failure"`
}

func (Failed) executionState() {}

type ExecutionSnapshot struct {
	ExecutionID   string         `json:"execution_id"`
	PlanReference PlanReference  `json:"plan_reference"`
	Revision      uint64         `json:"revision"`
	State         ExecutionState `json:"state"`
}

type ExecutionEvent interface{ executionEvent() }

type ExecutionStarted struct {
	EventID       string        `json:"event_id"`
	ExecutionID   string        `json:"execution_id"`
	PlanReference PlanReference `json:"plan_reference"`
	Input         Payload       `json:"input"`
}

func (ExecutionStarted) executionEvent() {}

type NodeAttemptSucceeded struct {
	EventID          string  `json:"event_id"`
	ExecutionID      string  `json:"execution_id"`
	ExpectedRevision uint64  `json:"expected_revision"`
	ActivationID     string  `json:"activation_id"`
	AttemptID        string  `json:"attempt_id"`
	AttemptNumber    uint32  `json:"attempt_number"`
	EffectID         string  `json:"effect_id"`
	NodeID           string  `json:"node_id"`
	Output           Payload `json:"output"`
}

func (NodeAttemptSucceeded) executionEvent() {}

type NodeAttemptFailed struct {
	EventID          string         `json:"event_id"`
	ExecutionID      string         `json:"execution_id"`
	ExpectedRevision uint64         `json:"expected_revision"`
	ActivationID     string         `json:"activation_id"`
	AttemptID        string         `json:"attempt_id"`
	AttemptNumber    uint32         `json:"attempt_number"`
	EffectID         string         `json:"effect_id"`
	NodeID           string         `json:"node_id"`
	Failure          AttemptFailure `json:"failure"`
}

func (NodeAttemptFailed) executionEvent() {}

type TimerFired struct {
	EventID           string `json:"event_id"`
	ExecutionID       string `json:"execution_id"`
	ExpectedRevision  uint64 `json:"expected_revision"`
	TimerID           string `json:"timer_id"`
	ActivationID      string `json:"activation_id"`
	NextAttemptNumber uint32 `json:"next_attempt_number"`
}

func (TimerFired) executionEvent() {}

type ExecutionEffect interface{ executionEffect() }

type ExecuteNodeAttempt struct {
	EffectID      string  `json:"effect_id"`
	ActivationID  string  `json:"activation_id"`
	AttemptID     string  `json:"attempt_id"`
	AttemptNumber uint32  `json:"attempt_number"`
	NodeID        string  `json:"node_id"`
	Input         Payload `json:"input"`
}

func (ExecuteNodeAttempt) executionEffect() {}

type ScheduleTimer struct {
	EffectID          string `json:"effect_id"`
	TimerID           string `json:"timer_id"`
	ActivationID      string `json:"activation_id"`
	FailedAttemptID   string `json:"failed_attempt_id"`
	NextAttemptNumber uint32 `json:"next_attempt_number"`
	DelayMs           uint64 `json:"delay_ms"`
}

func (ScheduleTimer) executionEffect() {}

type Transition struct {
	Snapshot ExecutionSnapshot
	Effects  []ExecutionEffect
}
