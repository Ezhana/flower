// Package witgen converts a WIT file into a Go bindings package
// targeting the wacogo host layer.
package witgen

import (
	"fmt"
	"os"
	"path/filepath"

	"go.bytecodealliance.org/wit"
)

// Load reads a WIT text or JSON file from path and returns the
// resolved AST. The wit package delegates WIT-text parsing to a
// vendored wasm-tools; JSON files are decoded directly. `use` and
// `include` references resolve before Load returns, so the *Resolve
// can be walked without further name lookup.
func Load(path string) (*wit.Resolve, error) {
	var (
		res *wit.Resolve
		err error
	)
	if filepath.Ext(path) == ".json" {
		res, err = wit.LoadJSON(path)
	} else {
		file, openErr := os.Open(path)
		if openErr != nil {
			return nil, fmt.Errorf("witgen: open %s: %w", path, openErr)
		}
		defer file.Close()
		res, err = wit.DecodeWIT(file)
	}
	if err != nil {
		return nil, fmt.Errorf("witgen: load %s: %w", path, err)
	}
	return res, nil
}
