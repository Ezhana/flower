# ADR-0002：原生 Rust 参考运行时，WASM Component 用于扩展节点

- Status: Superseded
- Date: 2026-08-21
- Superseded by: [ADR-0005](0005-portable-deterministic-engine-component.md)

## Context

Flower 同时需要跨平台、跨语言扩展、安全边界和成熟的系统集成。把完整流程引擎编译成 WASM 看似统一，但调度、数据库驱动、线程、异步运行时、文件系统、网络、调试和原生库集成都会受到不必要限制。

另一方面，直接 `dlopen` 用户提供的原生库会共享宿主进程权限，跨 OS/CPU 打包复杂，且 C ABI 很难自然表达 record、variant、result 和 list 等类型。

## Decision

首个参考架构采用：

```text
Native Rust Flower Runtime
            │
     WIT / Component Model
            │
   sandboxed extension nodes
```

- Rust Runtime 负责调度、持久化、恢复、权限和副作用；
- 扩展 Node 优先编译为 WebAssembly Component；
- WIT 定义 Component import/export contract；
- WASI 仅按能力显式授予；
- 核心逻辑与公共语义不得依赖 Wasmtime 私有类型；
- 原生 Node 和远程 Worker 仍是合法 backend，只要通过相同 conformance rules。

这不是“Rust 与 WASM 二选一”。Rust 是参考实现语言；WASM Component 是扩展的分发与隔离格式。

## Consequences

正面影响：

- Host 保留成熟的 Rust 生态和完整系统集成能力；
- 用户扩展获得跨平台格式、类型化 ABI 和更强隔离；
- Rust、Go、C#、Python 或 JavaScript guest 可以共享同一份 WIT contract；
- 能按 import 精确授予 log、http、secret 等能力。

代价与约束：

- Runtime 需要嵌入 Component-capable WASM engine；
- 各 guest 语言工具链成熟度不同；
- 调试跨 Host/guest 边界比纯原生代码复杂；
- WIT package 和 guest SDK 必须独立版本化。

## Rejected alternatives

### 完整 Runtime 编译成 WASM

拒绝作为首要架构。它把最依赖操作系统和持久化的部分放到了约束最多的边界，收益与复杂度不匹配。

### 所有插件使用原生动态库

拒绝作为默认扩展机制。平台矩阵、内存所有权和宿主权限风险不可接受。

### 用 WIT 暴露完整 Host API

拒绝。Component 应获得窄且可审计的能力，不应看到数据库、调度器和内部存储模型。
