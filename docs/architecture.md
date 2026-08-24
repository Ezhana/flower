# Flower 架构

## 定位

Flower 是可移植的纯解释器，不是工作流服务。Rust 实现定义行为，WIT 定义调用形状，SDK 调用同一份编译后的 Component。

```text
Rust caller ---------------------> flower-engine

Go / other language
    -> SDK -> WebAssembly Component -> flower-engine
```

不同语言 SDK 不复制编译和状态转换逻辑，因此仓库不维护跨实现的通用工作流语义规范。

## 核心模块

`flower-engine` 是唯一领域 crate，内部按职责分为：

```text
compiler.rs   definition -> validated linear plan
plan.rs       immutable plan, retry policy, plan fingerprint
execution.rs  snapshot + event -> snapshot + effects
identifier.rs deterministic activation, attempt, timer and effect identities
```

这些模块属于一个发布单元。它们不是独立产品边界，不再拆成互相依赖的微型 crate。

`flower-component` 只负责 WIT 类型与 Rust 类型之间的显式转换。Component 不导入 WASI，也不持有跨调用状态。

## 编译

Compiler 接收 `WorkflowDefinition`，验证：

- 唯一节点与边身份；
- 恰好一个 Start 和 Finish；
- edge 端点存在；
- 无分支、无环、无断连；
- Activity retry policy 合法。

成功后按 Start 到 Finish 的顺序生成 `ExecutableWorkflowPlan`。Plan fingerprint 覆盖 workflow ID、完整节点顺序、节点类型和 retry policy；任何会改变执行行为的计划字段都必须改变 fingerprint。

## 状态转换

唯一执行入口是：

```text
transition(plan, optional snapshot, event) -> transition result | structured error
```

转换结果包含下一份完整 Snapshot 和零个或多个 Effect。Payload 是不透明的 `media_type + bytes`，解释器只传递，不解析业务字段。

Revision、ActivationId、AttemptId、TimerId 和 EffectId 用于检查输入是否对应当前状态。EffectId 使用确定性哈希派生，使调用方可以在外部执行层实现幂等。

## 不属于 Flower 的职责

以下能力必须由调用方选择和实现：

- Snapshot 保存与加载；
- EventId 分配与重复事件检查；
- 事务、event log、outbox 与 lease；
- Effect 调度和 Worker 生命周期；
- timer clock；
- HTTP、数据库、文件系统和 secret 权限；
- 服务端、CLI 与部署方式。

把这些能力重新加入 `flower-engine` 会破坏纯函数边界，除非产品定位再次发生明确变化，否则不得添加。
