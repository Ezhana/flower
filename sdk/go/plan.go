package flower

type SpecificationVersion struct {
	Major uint16 `json:"major"`
	Minor uint16 `json:"minor"`
}

type PlanNode struct {
	ID   string   `json:"id"`
	Kind NodeKind `json:"kind"`
}

type ExecutableWorkflowPlan struct {
	SpecificationVersion SpecificationVersion `json:"specification_version"`
	WorkflowID           string               `json:"workflow_id"`
	Fingerprint          string               `json:"fingerprint"`
	Nodes                []PlanNode           `json:"nodes"`
}

type PlanReference struct {
	SpecificationVersion SpecificationVersion `json:"specification_version"`
	WorkflowID           string               `json:"workflow_id"`
	Fingerprint          string               `json:"fingerprint"`
}

func (p ExecutableWorkflowPlan) Reference() PlanReference {
	return PlanReference{
		SpecificationVersion: p.SpecificationVersion,
		WorkflowID:           p.WorkflowID,
		Fingerprint:          p.Fingerprint,
	}
}
