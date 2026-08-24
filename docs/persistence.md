# 持久化集成

## 最小调用

不需要崩溃恢复时，调用方只需在内存中保留最新 Snapshot：

```text
result = transition(plan, snapshot, event)
snapshot = result.snapshot
execute(result.effects)
```

Flower 本身不需要数据库。

## 只保存状态

如果只要求进程重启后继续，可以在每次成功转换后保存最新 Snapshot。此模式不能保证外部 Effect 不丢失或不重复，适合可重算、无关键副作用的任务。

## 可靠 Effect 投递

需要可靠外部操作时，调用方必须在一个原子提交中保存：

```text
previous revision
accepted event identity
new snapshot
emitted effect intents
```

提交后再投递 Effect。外部执行器必须按 EffectId 幂等，因为进程可能在执行成功后、记录确认前退出。

这通常需要调用方自己的乐观 revision、event deduplication 和 transactional outbox。Flower 不提供通用 Store trait 或数据库 schema，因为这些选择必须服从宿主应用的数据模型、事务范围和吞吐要求。

## Event Log

Event Log 是可选能力：

- 只继续执行：保存最新 Snapshot 即可；
- 需要审计、调试或 replay：额外保存 Event；
- 需要可靠副作用：再增加 outbox；
- 需要 exactly-once 业务效果：依赖目标系统幂等或业务补偿，解释器无法凭空提供。

不要把数据库实现塞回纯解释器来掩盖宿主责任。持久化存在于系统层，但不属于 Flower Engine crate。
