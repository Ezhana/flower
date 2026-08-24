# ADR-0002：解释器无状态，持久化由调用方负责

- Status: Accepted
- Date: 2026-08-24

## Context

短生命周期计算不需要数据库；durable workflow 又必须服从宿主应用自己的事务、事件身份、outbox 和部署模型。把 Store、dispatcher 和 lease 固化进解释器会扩大接口并制造低质量的通用抽象。

## Decision

Flower 只实现纯转换：

```text
plan + optional snapshot + event -> snapshot + effects
```

解释器不保存 Snapshot、不分配 EventId、不执行 Effect、不读取 clock。调用方按自己的可靠性要求选择内存状态、Snapshot 存储、event log 或 transactional outbox。

## Consequences

Flower 可以嵌入同步程序、服务端或 durable host，而不依赖任何一种数据库。代价是可靠投递和重复事件处理必须由集成方明确实现，不能由 Flower 的接口承诺兜底。
