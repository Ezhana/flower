# ADR-0005：将确定性执行内核作为可移植 Component

- Status: Accepted
- Date: 2026-08-23
- Supersedes: [ADR-0002](0002-native-rust-core-wasm-components.md)

## Context

ADR-0002 正确拒绝了“把数据库、调度、恢复和系统集成都塞进 WASM”，但错误地把这一结论扩大成了“WIT 只能描述扩展 Node”。这混淆了完整 Runtime 与确定性执行内核。

Flower 的核心状态转换是纯计算：

```text
validated workflow + execution snapshot + recorded event
                         │
                         ▼
                 new snapshot + effects
```

它不需要数据库、网络、时钟、线程或文件系统。Go 等宿主如果只能通过 C ABI 或进程外 API 调用这部分逻辑，就会额外承担内存所有权、平台打包或网络部署成本，也无法直接复用 Component Model 的 record、enum、list、option 与 result。

## Decision

Flower 采用双目标执行内核：

1. `flower-runtime` 保持平台无关、无 I/O，原生 Rust Host 可直接链接；
2. `flower-component` 将同一执行内核编译成无 WASI import 的 WebAssembly Component；
3. `flower:engine/workflow-engine@0.1.0` 只暴露确定性状态转换，不在 Component 内持有 durable state；
4. Go Host 负责保存 `execution-snapshot`、执行 `execute-node-effect`，再把完成事实送回内核；
5. 扩展 Node 的 WIT world 与应用调用执行内核的 WIT world 必须分开版本化，不能共享一个含混的 Host API；
6. WIT 仍然只是 Binding。公共语义由规范、IR 和 conformance cases 定义。

组件通过 `wasm32-unknown-unknown` 生成 core module，再根据 `wit-bindgen` 嵌入的 Component Type 用 `wasm-tools component new` 封装。确定性内核不得因为构建目标而获得 WASI 权限。

## Consequences

正面影响：

- Rust 与 Go 使用同一份状态转换实现，不复制流程语义；
- Component 边界天然支持结构化 WIT 类型，不暴露 Rust 布局；
- Host 可以在每次转换后持久化 snapshot 与 effect，符合 ADR-0003；
- Go SDK 隔离具体 Component runtime，未来可以替换实现而不改变领域 API。

代价与约束：

- 发布流程需要同时产出原生 crate 与 Component；
- 当前 Go Component runtime 生态仍在演进，SDK 必须固定依赖并用端到端测试守住 Canonical ABI；
- Component 内不能偷偷加入存储、网络调用或长期调度状态；这些能力属于 Host；
- Binding 兼容性与 Rust crate 版本必须分别管理。

## Rejected alternatives

### 完整 Runtime 编译成 WASM

继续拒绝。持久化、调度、恢复与系统集成不属于可移植确定性内核。

### Go 重新实现状态机

拒绝。两套实现会立即制造语义漂移，并让 conformance suite 在语义尚未稳定时承担不必要的兼容成本。

### 用 JSON 字符串穿过 WIT

拒绝。它放弃 WIT 的类型系统，把错误推迟到运行时，并产生无法独立演进的隐式 schema。
