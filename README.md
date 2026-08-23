# Flower

Flower 是语言无关、可持久化、可重放的工作流执行规范及 Rust 参考实现。当前 v0.1 内核只支持一条确定的 `start -> activity* -> finish` 路径；gateway、edge condition、失败、重试、取消与超时尚未进入规范。

## 架构

```text
WorkflowDefinition
       │
flower-compiler ── diagnostics + normalize
       │
ExecutableWorkflowPlan
       │
flower-kernel ── snapshot + event -> snapshot + effects
       │
flower-host ── atomic commit + outbox
       │
flower-store-memory
```

- `flower-plan` 只包含强类型身份、规范版本和不可变执行计划；
- `flower-compiler` 是定义进入 runtime 的唯一入口；
- `flower-kernel` 是无 I/O、无随机数、无 async runtime 的纯确定性函数；
- `flower-host` 协调 snapshot、事件和 Effect intent 的原子提交；
- `flower-component` 是 WIT 转换层，不保存 execution，也不导入 WASI；
- Go SDK 公开惯用类型，把生成的 WIT ABI 与 Component runtime 隔离在 `internal/componentabi`。

完整边界见 [架构说明](docs/architecture.md)，v0.1 行为见 [语义规范](spec/v0.1/semantics.md)。

## 核心语义

```text
None + ExecutionStarted
  -> AwaitingNode(first activity) + ExecuteNode

AwaitingNode + matching NodeCompleted
  -> AwaitingNode(next activity) + ExecuteNode
  -> or Completed + no effects
```

Payload 是不透明的 `{ media_type, bytes }`。Kernel 不解析 JSON。`ExecutionId` 与 `EffectId` 全局唯一，`EventId` 在单个 Execution 内唯一；`ExecutionStarted`、snapshot 与 store head 绑定同一个不可变 `PlanReference`。Effect ID 使用规范定义的域分隔、长度前缀 SHA-256 编码确定性推导，Kernel 不生成随机数。

共享 fixture 位于 `spec/v0.1/fixtures`，格式由 `spec/v0.1/fixture-schema.json` 固定。Rust 与 Go Component runner 逐项比较完整 plan、diagnostics、snapshot、effects 和 error。

## 验证

安装 Rust stable、`wasm32-unknown-unknown` target 与 Go 1.25.5+，然后运行：

```bash
scripts/verify.sh
```

脚本执行格式、Clippy、Rust tests、Component 构建/验证、无 WASI import 检查和 Go conformance tests。Component 构建使用仓库内的 `flower-component-tools`，不依赖全局安装 `wasm-tools`。

## 文档

- [架构说明](docs/architecture.md)
- [核心概念](docs/concepts.md)
- [规范组织](docs/specification.md)
- [路线图](docs/roadmap.md)
- [ADR](docs/adr/README.md)
