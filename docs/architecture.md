# Flower 架构

本文描述 Flower 的目标架构和模块边界。除“当前状态”一节外，内容代表设计方向，不代表仓库已经实现相应能力。

## 当前状态

截至当前版本：

- `flower-ir` 已实现带强类型标识的线性 Workflow aggregate，并在构造时验证结构不变量；
- `flower-runtime` 已实现 `start -> activity* -> finish` 的确定性 Event/Effect 状态转换与原生 runner；
- `flower-component` 通过 `flower:engine/workflow-engine@0.1.0` 导出同一内核；
- Go SDK 已能加载 Component、推进节点完成事件，并有跨 Canonical ABI 的端到端测试；
- gateway、edge condition、持久化、重试、取消、YAML schema 与通用调度器尚未实现。

因此，当前最重要的产物是可审查的边界和可执行的语义测试，而不是扩展更多目录。

## 架构原则

### 语义先于实现

Flower 的事实来源按以下顺序组织：

```text
machine-readable model + normative semantics + conformance suite
                              │
                              ▼
                    reference implementations
                              │
                              ▼
                      bindings and SDKs
```

Rust 类型、WIT、C 头文件或 RPC schema 都不能单独定义 Flower 语义。

### 宿主掌握事实与权限

运行时 Host 负责：

- 持久化执行状态与事件；
- 调度节点和管理 Attempt；
- 授予、拒绝和审计能力；
- 执行并记录副作用；
- 管理取消、超时、重试与恢复；
- 加载原生节点、WASM Component 或远程 Worker。

节点负责领域计算并返回结果或 Effect 请求。节点不拥有“未声明的默认权限”。

### 扩展边界与调用边界分离

“其他语言的应用调用 Flower”和“其他语言编写 Flower 节点”不是同一个问题：

```text
application -> Flower runtime
    使用 Rust API、C ABI 或进程外 API

Flower runtime -> extension node
    使用原生接口、WIT/WASM Component 或远程 Worker 协议
```

试图用同一套 ABI 同时解决两者，会让生命周期、错误、异步和权限模型混在一起。

## 逻辑分层

### 1. Specification

定义公共 Value Model、Execution Model、Error Model、Capability Model、版本规则和规范性行为。详见 [规范组织](specification.md)。

### 2. IR 与 Compiler

YAML、JSON 或未来的 GUI 都只是输入前端：

```text
YAML / JSON / other frontends
             │
          parse
             ▼
       source model
             │
   validate + normalize
             ▼
        Flower IR
             │
      generate bindings,
      schemas and fixtures
```

IR 应表达已经验证并规范化的语义，不保留某种源格式的偶然细节。Compiler 是 schema compiler 与接口生成器，不是通用语言编译器。

### 3. Rust Reference Runtime

Rust 运行时是首个可执行的规范解释器。它应该依赖 Flower IR，而不是反过来让 IR 复制 Rust 内部结构。

建议依赖方向：

```text
flower-cli -> flower-compiler -> flower-ir
           -> flower-runtime  -> flower-ir

bindings/adapters -> flower-runtime
flower-ir         -> no runtime or binding dependency
```

### 4. Execution Backends

节点实现可以来自：

- 内置或原生 Rust 节点；
- WebAssembly Component 节点；
- 本地子进程；
- 容器或远程 Worker；
- Human、Agent、Tool、Policy 等上层 Actor 适配器。

Backend 不得改变节点状态、重试或错误的公共含义。

### 5. Bindings

Flower 支持或计划支持三类互补绑定：

| 绑定 | 主要用途 | 特点 |
| --- | --- | --- |
| Native / C ABI | 同进程宿主应用 | 低开销；需处理平台打包和内存所有权 |
| WIT / Component | 跨语言扩展节点；嵌入确定性内核 | 类型化、可移植、能力受控；不同 world 独立版本化 |
| HTTP / RPC | 进程外客户端或 Worker | 覆盖面广；引入部署与网络边界 |

## 执行内核

目标模型不是“把长任务一直留在某段内存里”，而是事件驱动的状态转换：

```text
persisted state + recorded event
               │
               ▼
        transition decision
               │
               ▼
      new state + effects[]
               │
               ▼
      atomically persist intent
               │
               ▼
       host executes effects
```

WASM memory、Rust future 或某个进程都不是持久化边界。持久状态必须能够在进程退出后恢复。

## WebAssembly、WIT 与 WASI

Flower 对三者的定位不同：

- WebAssembly 是可移植且隔离的执行格式；
- Component Model 提供组件组合和 Canonical ABI；
- WIT 描述 Component 的 import/export 类型边界；
- WASI 提供时钟、文件、网络等标准 Host 能力。

完整 Host 的首选形态仍是 Native Rust。无 I/O 的确定性状态转换内核同时发布为 Component，供 Go 等 Host 嵌入；数据库、调度、恢复和系统集成不得进入该 Component。节点通常应优先依赖 Flower 定义的窄能力，例如 `flower:log`、`flower:http`、`flower:secret`，由 Host 记录其结果；只有无需纳入确定性记录的能力才考虑直接授予 WASI。详见 [ADR-0005](adr/0005-portable-deterministic-engine-component.md)。

## Agent Workflow Profile

Agent 编排是 Flower 的重要使用场景，但不应侵入通用核心。上层 profile 可将 Node 关联到：

- `Actor`：Human、Agent、Tool、Policy 或 Composite；
- `ContextPolicy`：本次执行可读取哪些任务状态、工件、代码和记忆；
- `CapabilityPolicy`：本次执行允许进行哪些外部操作；
- `OutputContract`：结构化输出与 verdict。

典型流程不是 `Review -> Finish`：

```text
Plan -> ApproveExecution -> Execute (dynamic fan-out) -> Review
                                                        ├─ approve -> Acceptance -> Finalize
                                                        ├─ request-changes -> Rework -> Execute
                                                        ├─ spawn-work -> Execute
                                                        ├─ escalate -> Decision
                                                        └─ reject -> Cancelled
```

长期记忆属于可替换的 `ContextProvider`，不属于固定节点类型。读写权限应分开配置，写入长期记忆需要独立提交策略。该方向仍是 proposed decision，见 [ADR-0004](adr/0004-agent-workflows-as-profile.md)。

## 明确不做的事

Flower 核心不提供任意函数、循环、递归、宏、类或通用表达式系统。业务逻辑属于实现语言；Flower 只定义逻辑如何声明、组合、执行和观察。

Flower 也不会重新实现 Rust、WASM、WASI、Protobuf、gRPC、HTTP 或容器运行时。它只在需要时为这些技术定义清晰的适配边界。
