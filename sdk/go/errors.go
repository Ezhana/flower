package flower

import "fmt"

type EngineError struct {
	Code    string
	Message string
}

func (e *EngineError) Error() string {
	return fmt.Sprintf("flower engine %s: %s", e.Code, e.Message)
}
