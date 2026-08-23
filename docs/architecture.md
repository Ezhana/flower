# Flower 架构

## 依赖方向

```text
compiler  -> plan
kernel    -> plan
host      -> kernel + plan
store     -> host
component -> compiler + kernel + plan
Go SDK    -> generated internal Component ABI
```

`flower-plan` 不依赖其他 Flower crate。`flower-kernel` 不依赖数据库、文件系统、Tokio、Wasm runtime 或 Host。仓库禁止引入没有明确领域职责的 common/utils crate。

## 定义与计划

`WorkflowDefinition` 保留输入结构，只能由 `flower-compiler` 消费。Compiler 检查唯一 ID、唯一 start/finish、edge endpoint、线性 degree、不连通节点与 cycle，再按 start-to-finish 路径生成稳定顺序和 SHA-256 fingerprint。

Runtime 只接收 `ExecutableWorkflowPlan`。计划反序列化后仍会执行完整性校验，因此伪造 fingerprint、拓扑或 specification version 会作为结构化错误返回。

## 确定性 Kernel

Kernel 的唯一入口是：

```text
transition(plan, optional snapshot, event) -> transition | structured error
```

Snapshot 只保存恢复所需的当前或最后一个 NodeActivation 与 Attempt，不保存增长的尝试或完成节点列表。完整历史属于 Event Log。开始事实与 snapshot 都绑定完整 `PlanReference`；replay 在消费事件前验证该绑定。Kernel 按规范顺序验证 execution ID、revision、activation、attempt、attempt number、effect、node 和 timer identity；任何外部输入错误均不得 panic。

Retry policy 在编译时固化进每个 Activity 的 PlanNode。Kernel 根据稳定 FailureCode 和整数 backoff 产生 `ScheduleTimer(delay_ms)`，不读取时钟。Host/timer service 只能通过持久化并提交 `TimerFired` 推动下一 Attempt；同一 activation 的每次重试都会产生新的 AttemptId 与 EffectId。

## Durable Host

Host 的执行顺序是：

```text
load snapshot
-> kernel transition
-> atomic commit(snapshot + event + effect intent + revision)
-> dispatch pending effect
-> record completion event
-> repeat
```

Host 把一次推进封装为单一 `ExecutionCommit`，Store 原子写入 store head、snapshot、event 与 outbox。`flower-store-memory` 与 `flower-store-postgres` 实现同一 optimistic revision contract。完全相同的 event 重投返回 `AlreadyCommitted`；同一 Execution 内复用 `EventId` 表示不同事实则拒绝；`EffectId` 在全局 outbox 中唯一。

Outbox entry 的持久状态是 `Pending | Claimed { claim_id, owner_id, lease_until } | Confirmed`。Dispatcher 必须先 claim 才能投递，并在 lease 有效期内携带完整 claim identity 确认；错误、已释放或过期 claim 不得确认。到期 claim 可被其他 owner 原子重领，主动 release 则立即回到 Pending。Host 的 lease clock 不进入 Kernel。

Effect dispatch 与 outbox 确认分离，语义仍是 at-least-once：进程可能在外部执行成功后、确认写入前退出，lease 到期后 Effect 会重投。外部执行器必须按全局 `EffectId` 幂等，系统不宣称 exactly-once。

PostgreSQL adapter 在一个事务内提交 execution head、snapshot、event 和 outbox。`(execution_id, event_id)` 与 `(execution_id, revision)` 唯一，`effect_id` 全局唯一；claim 使用单条 `FOR UPDATE SKIP LOCKED` CTE/update。`ScheduleTimer` 复用通用 outbox，因此核心持久化层不额外创建 timer 领域状态或独立 timer 表。

## Component 与 Go

`flower-component` 导出 `flower:engine/workflow-engine@0.2.0` 的 `compile` 与 `transition`，不保存 execution，不导入 WASI。WIT package version 与 snapshot 中的 specification version 是独立概念。

Go ABI 由仓库内固定的 `sdk/go/tools/wacogo-witgen` 从 WIT 一次生成。该生成器修复了 nominal 类型依赖排序和 Canonical ABI 间接参数 lowering；产物不再经过字符串修补，也不存在反射 ABI 桥。公开 SDK 不暴露 `wacogo.Val` 或 Component runtime，并在进入 ABI 前拒绝非法公共 enum 值。

## 明确不支持

v0.2 不支持 gateway、edge condition、Activity 多 outgoing edge、取消或超时。Worker 只能报告稳定 FailureCode 和详情，不能决定 Kernel 策略；不能用放宽线性拓扑约束伪装 gateway 支持。
