# ADR-0001：WIT 接口与单一 Rust 行为实现

- Status: Accepted
- Date: 2026-08-24

## Context

统一所有工作流语义并要求多个语言独立实现既不现实，也会迫使项目长期维护抽象规范和兼容性测试。WIT 能定义 ABI 类型，却不能单独保证不同实现具有相同行为。

## Decision

- `flower-engine` 是编译和状态转换行为的唯一实现；
- WIT 定义 `compile` 与 `transition` 的跨语言调用接口；
- 非 Rust SDK 加载同一份 Rust WebAssembly Component；
- fixtures 验证原生调用和 Component 调用一致，不认证第三方实现；
- 核心代码保持一个 crate，不按概念拆成多个发布单元。

## Consequences

行为修改只需要维护一套实现。调用接口仍可跨语言复用，但第三方重写只能获得 ABI 形状兼容，不自动获得行为兼容。
