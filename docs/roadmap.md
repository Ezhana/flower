# Flower 路线图

路线图按领域依赖排序，不按功能演示顺序排序。后续阶段不得反向修改已经封板的身份、生命周期或持久化契约。

## 阶段 0：真正封板 v0.1

目标是把线性成功路径变成可信地基。阶段 0 完成前，禁止引入失败、重试或 PostgreSQL。

封板范围：

- Effect 身份：`EffectId::derive_execute_node` 使用 `flower/effect/v1` domain separator、长度前缀字段和 SHA-256；`EffectId` 全局唯一，`EventId` 在单个 Execution 内唯一；共享 fixture 固定输入和确切输出。
- Plan 绑定：`PlanReference { specification_version, workflow_id, fingerprint }` 同时绑定 `ExecutionStarted`、snapshot、store head 和 event log；replay 在处理首个事件前验证完整引用。
- Conformance suite：版本化 JSON Schema；每个 step 提供完整输入 snapshot、event 和完整预期结果；精确比较 payload、node、revision、fingerprint、EffectId、顺序和错误；Rust/Go runner 只解释 fixture，不按 scenario 分支；元测试证明篡改任一 expected 值必然失败。
- Host/Store：以单一 `ExecutionCommit` 值对象表达原子提交；同一事件重投返回 `AlreadyCommitted`，同一 `EventId` 对应不同事实则拒绝；确认不存在的 Effect 必须报错；outbox 是 at-least-once，外部执行器按全局 `EffectId` 幂等。
- Go ABI：所有 public → ABI 转换显式返回错误；bindings 只由修复后的生成器产生；不存在 `fixbindings`；CI 重新生成 bindings 并检查 clean diff。

## 阶段 1：Node Activation 与 Attempt

失败是 Attempt 的结果，不能先于 Attempt 建模。领域层级固定为：

```text
Execution
└── NodeActivation
    └── Attempt
        └── ExecuteNodeAttempt Effect
```

新增身份和值：

- `NodeActivationId`：一次节点逻辑激活；
- `AttemptId`：该激活的一次实际尝试；
- `AttemptNumber`：从 1 开始；
- `FailureCode`：稳定、可匹配的失败分类代码。

事件：

```text
ExecutionStarted
NodeAttemptSucceeded
NodeAttemptFailed
```

Effect：

```text
ExecuteNodeAttempt {
  effect_id,
  activation_id,
  attempt_id,
  attempt_number,
  node_id,
  input
}
```

Snapshot 只保存当前 activation/attempt；不可覆盖的 Attempt 历史只属于 Event Log。

第一刀只允许：首次 Attempt 成功；首次 Attempt 失败后 Execution 进入 terminal `Failed`；错误的 attempt、effect、revision、activation 全部拒绝。此阶段不自动重试。

## 阶段 2：确定性重试与定时器边界

Retry policy 编译进 `ExecutableWorkflowPlan`，Kernel 根据 `FailureCode` 决策，绝不信任 Worker 提供 `retryable: bool`。

```text
NodeAttemptFailed
├── terminal / exhausted -> ExecutionFailed
└── retryable            -> WaitingForRetry + ScheduleTimer

TimerFired
└── AwaitingAttempt(next) + ExecuteNodeAttempt
```

时间属于 Host：Kernel 不读取时钟，只产生 `ScheduleTimer` intent；Host 原子持久化并调度；`TimerFired` 是记录后的输入事实；每次重试创建新的 `AttemptId`，绝不覆盖旧 Attempt。

## 阶段 3：持久化适配器

内存 Store 必须先通过以下行为测试：

- commit 前退出；
- commit 后、dispatch 前退出；
- dispatch 后、ack 前退出；
- 两个 dispatcher 同时 claim；
- duplicate event 重投；
- stale revision；
- retry timer 重投；
- 完整 replay 与 stored head 一致。

PostgreSQL 只能实现已经冻结的 Store contract，不得反向塑造领域模型。
