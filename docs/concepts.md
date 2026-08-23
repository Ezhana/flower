# Flower 核心概念

本文建立跨规范、实现和绑定共用的词汇。类型的最终字段仍需由 machine-readable model 与 conformance suite 冻结。

## 定义层

### Workflow

一组 Node、它们的连接关系、公共输入输出及执行策略。Workflow 描述允许发生什么，不承载某种语言的可执行函数体。

### Node

可独立调度和观察的一项工作。Node 定义输入、输出、错误与能力需求；Node implementation 才决定工作由 Rust、WASM、远程服务、人或 Agent 完成。

### Edge

Node 之间的路由关系。Edge 可以根据已经持久化的结构化结果或 verdict 选择下一步，但核心 schema 不应演变成任意表达式语言。

### Actor

执行 Node 的逻辑角色，例如 Human、Agent、Tool、Policy 或 Composite。Actor 是 Agent Workflow Profile 的概念，不应成为通用运行时对特定 AI 厂商的依赖。

### ContextPolicy 与 ContextProvider

`ContextPolicy` 声明 Node 可以看到哪些上下文。`ContextProvider` 负责实际提供任务状态、工件、代码索引或长期记忆。

长期记忆的读和写是不同权限。读取历史知识不意味着节点可以把未经确认的推断写成长期事实。

## 执行层

### Execution

Workflow 的一次逻辑执行，具有全局唯一且稳定的 `ExecutionId`。进程重启、节点推进、重试或 Worker 迁移不得改变该身份。

### NodeActivation

Execution 中某个 Node 的一次逻辑激活。循环或返工未来可以再次激活同一 Node，但必须创建新的 `NodeActivationId`。

### Attempt

NodeActivation 的一次实际执行尝试。首次执行也是 Attempt；成功和失败都是 Attempt 的结果。重试必须创建新的 `AttemptId` 和递增的 `AttemptNumber`，不能覆盖历史。

### State

运行时持久化的当前事实。首版最小状态集合预计包含 `Pending`、`Running`、`Completed`、`Failed` 和 `Cancelled`；合法转换必须由规范和测试共同定义。

### Event

已经发生并被记录的事实，例如输入到达、Attempt 完成、人工给出 verdict 或外部调用返回。Event 推动状态转换，不表达尚未执行的愿望。

`EventId` 的唯一作用域是单个 Execution。同一 Execution 内的同一 `EventId` 只能表示同一事实；完全相同的事实重投是幂等提交，内容不同则是身份冲突。

### Effect

状态转换要求 Host 执行的外部操作，例如调度 Node、发送 HTTP 请求、创建 Executor、等待信号或取消执行。Effect 是意图；只有执行结果成为 Event 后，才是新的事实。

`EffectId` 在所有 Execution 之间全局唯一。outbox 只提供 at-least-once 投递，因此外部执行器必须用 `EffectId` 去重，不能依赖进程内状态或“通常只调用一次”的假设。

### Artifact

流程产生或引用的持久化工件，例如 plan、diff、测试报告、review、日志或 snapshot。Artifact 应有稳定身份和来源，不能只存在于 Agent 的上下文窗口中。

## 合同层

### Payload

v0.1 只定义不透明的 `media_type + bytes`。Kernel 负责传递而不解析业务字段。JSON、CBOR 或领域 verdict 由 media type 标识；gateway 未来使用显式 RouteDecision，不能在 Kernel 中对任意 JSON 执行表达式。

### ExecutionError

跨语言错误，而不是 Rust panic、C# exception 或 HTTP status 的直接镜像。候选分类包括 `InvalidInput`、`Timeout`、`Unavailable`、`Cancelled`、`PermissionDenied` 和 `Internal`。

错误至少需要明确稳定 code、是否可重试、是否终止以及可公开信息。实现内部堆栈不得默认跨越公共边界。

### Capability

Node 对 Host 功能的显式需求，例如 log、http、kv、secret、clock、random、filesystem、database、event 或 subflow。声明需求不等于获得授权；Host 可以因为不支持、未授权或配置无效而拒绝实例化或执行。

### Binding

Flower 语义在特定互操作机制中的表示，例如 WIT、C ABI 或 RPC schema。Binding 可以有独立版本，但不能成为语义事实来源。

### Host

实例化运行时或 Node、提供能力并掌握系统权限的一方。Host 负责把 Effect 执行为外部操作并把结果记录为 Event。

## Review 与完成

Review 是产生 verdict 的工作，不是终止状态的别名。推荐的 verdict 至少能表达：

- `approve`：进入 Acceptance，而不是直接 Finish；
- `request-changes`：创建返工并重新执行；
- `partial-approve`：只返工未通过部分；
- `spawn-work`：增加新的执行分支；
- `escalate`：进入人工或策略决策；
- `reject`：按策略取消。

`Finish` 只应在所有验收条件、必要工件和终止不变量均满足后出现。
