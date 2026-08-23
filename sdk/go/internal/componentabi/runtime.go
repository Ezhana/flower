package componentabi

import (
	"context"
	"errors"
	"fmt"
	"io"

	"github.com/partite-ai/wacogo"
	"github.com/partite-ai/wacogo/host"

	abi "flower.dev/sdk/go/internal/componentabi/flower/engine/workflowengine"
)

type Client struct {
	runtime *wacogo.Engine
	root    *wacogo.ComponentInstance
	caller  *host.ComponentInstance
	factory *abi.Factory
	api     abi.WorkflowEngine
}

func Load(ctx context.Context, component io.Reader) (*Client, error) {
	runtime := wacogo.NewEngine(ctx)
	compiled, err := runtime.LoadComponent(ctx, component)
	if err != nil {
		_ = runtime.Close(ctx)
		return nil, fmt.Errorf("load flower component: %w", err)
	}
	root, err := compiled.Instantiate(ctx)
	if err != nil {
		_ = runtime.Close(ctx)
		return nil, fmt.Errorf("instantiate flower component: %w", err)
	}
	exported := root.ExportedInstance(abi.InterfaceName)
	if exported == nil {
		_ = root.Close(ctx)
		_ = runtime.Close(ctx)
		return nil, fmt.Errorf("flower component does not export %s", abi.InterfaceName)
	}
	factory, err := abi.NewFactory(ctx, runtime)
	if err != nil {
		_ = root.Close(ctx)
		_ = runtime.Close(ctx)
		return nil, fmt.Errorf("create generated ABI factory: %w", err)
	}
	caller, err := factory.NewInstance(ctx, unreachableEngine{}, nil)
	if err != nil {
		_ = factory.Close(ctx)
		_ = root.Close(ctx)
		_ = runtime.Close(ctx)
		return nil, fmt.Errorf("create generated ABI caller: %w", err)
	}
	return &Client{
		runtime: runtime,
		root:    root,
		caller:  caller,
		factory: factory,
		api:     abi.WrapInstance(caller, exported),
	}, nil
}

func (c *Client) Compile(ctx context.Context, definition abi.WorkflowDefinition) (abi.ResultExecutableWorkflowPlanListDiagnostic, error) {
	return c.api.Compile(ctx, definition)
}
func (c *Client) Transition(ctx context.Context, plan abi.ExecutableWorkflowPlan, snapshot abi.OptionExecutionSnapshot, event abi.ExecutionEvent) (abi.ResultTransitionResultEngineError, error) {
	return c.api.Transition(ctx, plan, snapshot, event)
}

func (c *Client) Close(ctx context.Context) error {
	if c.runtime == nil {
		return nil
	}
	err := errors.Join(c.caller.Close(ctx), c.factory.Close(ctx), c.root.Close(ctx), c.runtime.Close(ctx))
	c.runtime = nil
	return err
}

type unreachableEngine struct{}

func (unreachableEngine) Compile(context.Context, abi.WorkflowDefinition) (abi.ResultExecutableWorkflowPlanListDiagnostic, error) {
	return nil, errors.New("ABI caller implementation is unreachable")
}
func (unreachableEngine) Transition(context.Context, abi.ExecutableWorkflowPlan, abi.OptionExecutionSnapshot, abi.ExecutionEvent) (abi.ResultTransitionResultEngineError, error) {
	return nil, errors.New("ABI caller implementation is unreachable")
}
