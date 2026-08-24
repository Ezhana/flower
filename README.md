# Flower

Flower 是通过 Rust crate 与 WebAssembly Component 提供的确定性工作流解释器。它只完成两件事：把工作流定义编译成执行计划，以及根据当前快照和输入事件计算下一份快照与外部 Effect。

Flower 不提供 CLI、服务端、数据库适配器、Worker、timer service 或通用工作流规范。Rust 实现是行为的唯一事实来源；WIT 定义跨语言调用接口，SDK 负责加载同一份 Component，而不是重新实现状态机。

## 调用模型

```text
WorkflowDefinition --compile--> ExecutableWorkflowPlan

Plan + optional Snapshot + Event
                 --transition--> Snapshot + Effect[]
```

`transition` 是无 I/O、无时钟、无随机数、无异步运行时的纯函数。调用方执行 Effect，再把执行结果作为新的 Event 传回解释器。

当前实现支持确定的 `start -> activity* -> finish` 路径、显式 NodeActivation/Attempt，以及由 Plan 决定的确定性重试：

```text
None + ExecutionStarted
  -> AwaitingAttempt + ExecuteNodeAttempt

AwaitingAttempt + NodeAttemptSucceeded
  -> AwaitingAttempt + ExecuteNodeAttempt
  -> or Completed

AwaitingAttempt + NodeAttemptFailed
  -> Failed
  -> or WaitingForRetry + ScheduleTimer

WaitingForRetry + TimerFired
  -> AwaitingAttempt + ExecuteNodeAttempt
```

## 仓库结构

- `crates/flower-engine`：定义、编译、计划、状态转换和确定性身份；
- `crates/flower-component`：`flower:engine/workflow-engine@0.1.0` WIT 适配层；
- `crates/flower-component-tools`：Component 封装与验证工具；
- `crates/flower-engine-tests`：Rust 与 Go Component 共享的回归 fixture；
- `sdk/go`：加载 Rust Component 的 Go SDK；
- `wits/engine.wit`：跨语言接口；
- `fixtures`：接口级回归数据。

## 持久化边界

解释器不保存状态。短生命周期调用可以只把 Snapshot 放在内存中；需要崩溃恢复的调用方自行持久化 Snapshot。

如果调用方还要求可靠投递外部操作，则必须在自己的事务边界内原子保存 `new snapshot + accepted event identity + emitted effects`。EventId、去重、outbox、lease 和数据库 schema 都属于调用方，不属于 Flower 接口。

详见 [架构](docs/architecture.md) 与 [持久化集成](docs/persistence.md)。

## 开发

仓库使用 [just](https://github.com/casey/just) 管理开发命令：

```bash
just                 # 查看命令
just format          # 格式化 Rust 与 Go
just rust-test       # Rust 测试
just component       # 构建并验证 Component
just go-test         # Go SDK 与生成器测试
just verify          # 完整校验
```

可通过 `FLOWER_CARGO_BIN` 和 `FLOWER_GO_BIN` 指定工具路径。完整校验需要 Rust stable、`wasm32-unknown-unknown` target、Go 和 just。

## 文档

- [架构](docs/architecture.md)
- [接口边界](docs/interface.md)
- [核心概念](docs/concepts.md)
- [持久化集成](docs/persistence.md)
- [路线图](docs/roadmap.md)
- [架构决策](docs/adr/README.md)
