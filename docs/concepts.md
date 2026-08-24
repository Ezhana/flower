# Flower 核心概念

## WorkflowDefinition

调用方提交给 Compiler 的声明式输入。目前只表达线性 Start、Activity、Finish 节点和连接关系，不包含可执行函数体。

## ExecutableWorkflowPlan

Compiler 产生的规范化执行计划。Runtime 只接受 Plan，不直接解释未经验证的 Definition。Plan fingerprint 绑定所有会改变执行结果的内容，包括 retry policy。

## ExecutionSnapshot

一次 Execution 的完整可继续状态。Snapshot 是普通可序列化值，不暗示由 Flower 保存。调用方可以把它保存在内存、数据库、对象存储或自己的 event-sourced 系统中。

## Event

调用方交给解释器的已发生事实：开始执行、Attempt 成功、Attempt 失败或 timer 触发。Event 不包含调用方的幂等事件身份；重复请求处理属于调用方。

## Effect

解释器要求调用方执行的外部动作。目前是 `ExecuteNodeAttempt` 或 `ScheduleTimer`。Effect 是意图，不代表动作已经完成。

EffectId 是确定性的关联和幂等键。Flower 不提供投递队列，也不承诺 exactly-once。

## NodeActivation 与 Attempt

NodeActivation 表示某节点在一次 Execution 中的一次逻辑激活。Attempt 表示该 Activation 的一次实际执行。重试保留 ActivationId，创建新的 AttemptId、AttemptNumber 和 EffectId。

## Payload

Payload 是不透明的 `{ media_type, bytes }`。Flower 不解析 JSON、不做字段映射，也不执行表达式。

## Retry

Retry policy 是 Plan 的一部分。Worker 只报告稳定 FailureCode，不能通过运行时布尔值决定是否重试。解释器计算确定性的整数 `delay_ms`，调用方负责计时并提交 `TimerFired`。
