package flower

type PlanNode struct {
	ID          string       `json:"id"`
	Kind        NodeKind     `json:"kind"`
	RetryPolicy *RetryPolicy `json:"retry_policy"`
}

type ExecutableWorkflowPlan struct {
	WorkflowID  string     `json:"workflow_id"`
	Fingerprint string     `json:"fingerprint"`
	Nodes       []PlanNode `json:"nodes"`
}

type PlanReference struct {
	WorkflowID  string `json:"workflow_id"`
	Fingerprint string `json:"fingerprint"`
}

func (p ExecutableWorkflowPlan) Reference() PlanReference {
	return PlanReference{
		WorkflowID:  p.WorkflowID,
		Fingerprint: p.Fingerprint,
	}
}
