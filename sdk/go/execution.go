package flower

type Payload struct {
	MediaType string `json:"media_type"`
	Bytes     []byte `json:"bytes"`
}

type ExecutionState interface{ executionState() }

type AwaitingNode struct {
	NodeID   string  `json:"node_id"`
	EffectID string  `json:"effect_id"`
	Input    Payload `json:"input"`
}

func (AwaitingNode) executionState() {}

type Completed struct {
	Output Payload `json:"output"`
}

func (Completed) executionState() {}

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

type NodeCompleted struct {
	EventID          string  `json:"event_id"`
	ExecutionID      string  `json:"execution_id"`
	ExpectedRevision uint64  `json:"expected_revision"`
	EffectID         string  `json:"effect_id"`
	NodeID           string  `json:"node_id"`
	Output           Payload `json:"output"`
}

func (NodeCompleted) executionEvent() {}

type ExecutionEffect interface{ executionEffect() }

type ExecuteNode struct {
	EffectID    string  `json:"effect_id"`
	ExecutionID string  `json:"execution_id"`
	NodeID      string  `json:"node_id"`
	Input       Payload `json:"input"`
}

func (ExecuteNode) executionEffect() {}

type Transition struct {
	Snapshot ExecutionSnapshot
	Effects  []ExecutionEffect
}
