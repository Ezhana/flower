# Architecture Decision Records

ADR 记录影响多个模块、公共语义或长期演进的重要决定。ADR 一旦 Accepted，不通过新的 superseding ADR 删除或重写历史。

## 状态

- **Proposed**：方向可讨论，尚不能作为实现约束；
- **Accepted**：新实现必须遵守；
- **Deprecated**：仍保留历史，但不建议新增使用；
- **Superseded**：已由新的 ADR 替代，并链接替代者。

## 索引

| ADR | 状态 | 决策 |
| --- | --- | --- |
| [0001](0001-specification-first.md) | Accepted | 规范优先，语义不由实现或 binding 定义 |
| [0002](0002-native-rust-core-wasm-components.md) | Superseded | 原生 Rust 参考运行时，WASM Component 仅用于扩展节点 |
| [0003](0003-recorded-effect-boundary.md) | Accepted | Host 记录并执行副作用，状态转换不直接触碰外部世界 |
| [0004](0004-agent-workflows-as-profile.md) | Proposed | Agent 编排作为可组合 profile，不硬编码进通用核心 |
| [0005](0005-portable-deterministic-engine-component.md) | Accepted | 确定性执行内核同时提供原生 Rust 与 WIT Component 目标 |

## 新增 ADR

每份 ADR 至少包含：

```text
Title
Status
Context
Decision
Consequences
```

如果修改 Accepted 决策，应新增 ADR，并在新旧文档中互相标注 `Supersedes` / `Superseded by`。
