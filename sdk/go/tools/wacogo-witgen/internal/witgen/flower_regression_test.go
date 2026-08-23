package witgen

import (
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestFlowerNominalOrderAndIndirectParameters(t *testing.T) {
	directory := t.TempDir()
	witPath := filepath.Join(directory, "fixture.wit")
	witSource := `package test:fixture;

interface api {
    record payload { value: string }
    variant choice { payload(payload) }
    record envelope { choice: choice }

    call: func(a: envelope, b: envelope, c: envelope, d: envelope, e: envelope, f: envelope) -> result<envelope, string>;
}

world client { import api; }
`
	if err := os.WriteFile(witPath, []byte(witSource), 0o600); err != nil {
		t.Fatal(err)
	}
	files, err := Generate(Options{
		WitPath:     witPath,
		World:       "test:fixture/client",
		OutDir:      directory,
		PackageRoot: "test.dev/generated",
		DryRun:      true,
	})
	if err != nil {
		t.Fatal(err)
	}

	bind := string(files[filepath.Join("test", "fixture", "api", "api.bind.go")])
	payloadIndex := strings.Index(bind, "typPayload_ :=")
	choiceIndex := strings.Index(bind, "typChoice_ :=")
	envelopeIndex := strings.Index(bind, "typEnvelope_ :=")
	if payloadIndex < 0 || choiceIndex < payloadIndex || envelopeIndex < choiceIndex {
		t.Fatalf("nominal declarations are not in dependency order")
	}

	wrapped := files[filepath.Join("test", "fixture", "api", "api.wrap.go")]
	if !strings.Contains(string(wrapped), "paramsPtr_, err_ := callee.Realloc") {
		t.Fatalf("oversized parameter tuple was not lowered indirectly")
	}
	if !strings.Contains(string(wrapped), "lowerMemEnvelope") {
		t.Fatalf("indirect parameter fields do not use generated memory lowering")
	}
	if _, err := parser.ParseFile(token.NewFileSet(), "api.wrap.go", wrapped, parser.AllErrors); err != nil {
		t.Fatalf("generated wrapper is invalid Go: %v", err)
	}
}
