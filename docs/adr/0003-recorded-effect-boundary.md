# ADR-0003：Host 记录并执行副作用

- Status: Accepted
- Date: 2026-08-21

## Context

长时间运行的 Workflow 必须处理进程退出、重试、超时、人工等待和 Worker 迁移。HTTP、数据库、时钟、随机数、文件系统、外部事件和 LLM 调用如果在重放时再次执行，会产生重复扣款、重复消息、不同路由结果或不可解释的状态分叉。

单纯依靠 WASM sandbox 不能解决这个问题；它控制“能不能访问”，不定义“何时记录、是否重放和如何恢复”。

## Decision

Flower 将状态决策与副作用执行分离：

```text
State + Event -> Transition(State, Effect[])
```

Runtime 必须先持久化新的状态和 Effect intent，再由 Host 执行 Effect。执行结果以带关联标识的 Event 记录，然后驱动下一次转换。

同一个 Effect 必须有稳定身份或幂等键。恢复与重放默认返回已记录结果，不重复执行已经确认的外部操作。无法提供 exactly-once 的外部系统必须明确采用 at-least-once 或补偿语义，不能伪装成 exactly-once。

Component Node 需要外部能力时，应优先调用 Flower Host capability，由 Host 完成记录。直接授予 `wasi:http`、clock、random 或 filesystem 只适用于规范明确允许非确定性的场景。

## Consequences

正面影响：

- 可以在进程重启后恢复；
- 重试与重放具有可解释行为；
- 外部操作具备审计轨迹；
- 测试可以用 deterministic Effect handler 替代真实系统；
- 能统一处理 Human、Agent、Tool 和远程 Worker 的异步完成事件。

代价与约束：

- Runtime 需要 event/effect 持久化和关联模型；
- Node API 不能把任意 I/O 当作透明普通函数；
- 事务边界、outbox、幂等和补偿需要明确规范；
- 流式输出等场景需要定义增量事件，而不是绕过记录层。

## Rejected alternatives

### Node 直接执行任意 I/O

拒绝。该模型无法可靠重放，也无法统一授权、观测和测试。

### 仅在失败后依赖业务补偿

拒绝作为通用语义。补偿是某些 Effect 的策略，不是缺少稳定身份和执行记录的借口。

### 将 WASM memory 作为运行状态

拒绝。WASM memory 是计算资源，不是 durable persistence boundary。
