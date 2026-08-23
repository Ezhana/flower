package flower

type NodeKind string

const (
	StartNode    NodeKind = "start"
	ActivityNode NodeKind = "activity"
	FinishNode   NodeKind = "finish"
)

type NodeDefinition struct {
	ID   string   `json:"id"`
	Kind NodeKind `json:"kind"`
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
