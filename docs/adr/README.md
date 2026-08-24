# Architecture Decision Records

ADR 只记录当前架构仍然依赖的决策。仓库没有兼容包袱，已经失效的规范优先、内置 Host/Store 和 CLI 方向不作为历史约束保留。

| ADR | 状态 | 决策 |
| --- | --- | --- |
| [0001](0001-single-rust-engine.md) | Accepted | WIT 接口与单一 Rust 行为实现 |
| [0002](0002-pure-interpreter-boundary.md) | Accepted | 解释器无状态，持久化由调用方负责 |
