package flower

type NodeKind string

const (
	StartNode    NodeKind = "start"
	ActivityNode NodeKind = "activity"
	FinishNode   NodeKind = "finish"
)

type NodeDefinition struct {
	ID          string       `json:"id"`
	Kind        NodeKind     `json:"kind"`
	RetryPolicy *RetryPolicy `json:"retry_policy"`
}

type BackoffKind string

const (
	NoBackoff          BackoffKind = "none"
	FixedBackoff       BackoffKind = "fixed"
	ExponentialBackoff BackoffKind = "exponential"
)

type BackoffPolicy struct {
	Type           BackoffKind `json:"type"`
	DelayMs        uint64      `json:"delay_ms,omitempty"`
	InitialDelayMs uint64      `json:"initial_delay_ms,omitempty"`
	Multiplier     uint32      `json:"multiplier,omitempty"`
	MaximumDelayMs uint64      `json:"maximum_delay_ms,omitempty"`
}

type RetryPolicy struct {
	MaxAttempts           uint32        `json:"max_attempts"`
	RetryableFailureCodes []string      `json:"retryable_failure_codes"`
	Backoff               BackoffPolicy `json:"backoff"`
}

type EdgeDefinition struct {
	ID     string `json:"id"`
	Source string `json:"source"`
	Target string `json:"target"`
}

type WorkflowDefinition struct {
	ID    string           `json:"id"`
	Nodes []NodeDefinition `json:"nodes"`
	Edges []EdgeDefinition `json:"edges"`
}

type Diagnostic struct {
	Code    string  `json:"code"`
	Message string  `json:"message"`
	Subject *string `json:"subject"`
}
