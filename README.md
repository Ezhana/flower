# Flower

Flower 是一套面向可移植工作流运行时的、语言与平台无关的执行规范，以及该规范的 Rust 参考实现。

项目仍处于架构探索期。当前仓库只有 workspace、示例流程和模块占位代码，尚不具备可用的解析、执行、WASM、FFI 或持久化能力。现阶段的首要目标是收敛最小语义核心，而不是承诺完整 API。

## 核心定位

Flower 将稳定语义与具体实现技术分开：

- Flower Specification 定义值、节点、执行、错误、能力和状态转换的含义；
- Rust 是首个参考实现，不是规范本身；
- WebAssembly Component Model 是隔离并分发跨语言节点的首选格式；
- WIT 是 Component Model 的绑定，不是 Flower 的语义来源；
- WASI 是可选的标准能力集合，不会默认授予节点完整系统权限；
- C ABI 与进程外 API 面向“应用调用运行时”，WIT 面向“运行时加载扩展”，两类边界解决不同问题；
- 规范文档解释语义，一致性测试验证实现。

```text
                      Flower Specification
                    semantic model + rules
                               │
                     Rust reference runtime
                               │
              ┌────────────────┼────────────────┐
              │                │                │
        native binding   component binding  service binding
            C ABI           WIT / WASM        HTTP / RPC
              │                │                │
        host applications  sandboxed nodes   remote clients
```

详细边界见 [架构说明](docs/architecture.md) 和 [ADR 索引](docs/adr/README.md)。

## 最小领域模型

| 概念 | 含义 |
| --- | --- |
| Workflow | 节点、连接关系、输入输出及执行策略的声明 |
| Node | 一项可调度工作；描述契约，不绑定实现语言 |
| Execution | 某个节点或工作流的一次有稳定标识的执行 |
| Attempt | Execution 的一次具体尝试，重试会产生新 Attempt |
| Event | 推动状态机前进的已记录事实 |
| Effect | 运行时根据决策执行的外部操作请求 |
| Capability | 节点显式申请、由 Host 决定是否授予的能力 |
| Artifact | 计划、代码差异、测试结果、审查意见等可追踪产物 |

完整术语见 [核心概念](docs/concepts.md)。

## 为什么把计算和副作用分开

Flower 的长期执行模型以确定性状态转换为中心：

```text
current state + event -> transition(new state, effects[])
```

网络、数据库、时钟、随机数、文件系统、外部事件和 LLM 调用都属于副作用。节点不应把这些操作伪装成可重放的普通函数；Host 负责记录请求与结果，恢复或重放时复用已记录结果。该决策详见 [ADR-0003](docs/adr/0003-recorded-effect-boundary.md)。

## 仓库结构

```text
crates/
  flower-ir/        # 计划中的语言无关 IR
  flower-compiler/  # 计划中的解析、验证与生成管线
  flower-runtime/   # 计划中的 Rust 参考运行时
  flower-cli/       # 计划中的命令行入口
docs/               # 架构、规范、路线图与 ADR
examples/           # 流程定义示例
ffi/                # 预留的原生 C ABI
sdk/                # 预留的语言 SDK
wits/               # 预留的 WIT binding
```

以上目录中的“计划中”不是完成状态。以 [路线图](docs/roadmap.md) 的验收条件为准。

## 代码风格与质量检查

仓库使用 Rust 2024 edition，并通过根目录的 [`rustfmt.toml`](rustfmt.toml) 统一格式。安装包含 `rustfmt` 和 `clippy` 的稳定 Rust 工具链后依次运行：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

`rustfmt` 负责确定性的代码排版；Clippy 负责可疑写法和惯用法检查；测试负责行为验证。格式问题使用 `cargo fmt --all` 自动修复，Clippy 告警必须通过代码修改或有明确依据的局部 `#[allow(...)]` 解决，不能在 workspace 级别静默关闭。

当前检查只能验证 workspace 占位 crate 的工程基线，不能证明 Flower 的目标语义已经实现。

示例流程位于 [`examples/hello.flower.yaml`](examples/hello.flower.yaml)。它目前用于讨论输入格式，不代表已经冻结的 schema。

## 文档

- [架构说明](docs/architecture.md)：系统边界、依赖方向与目标运行模型；
- [核心概念](docs/concepts.md)：跨语言公共词汇和状态语义；
- [规范组织](docs/specification.md)：机器模型、规范文本和一致性测试如何共同构成规范；
- [路线图](docs/roadmap.md)：按可验证结果推进的实现顺序；
- [ADR](docs/adr/README.md)：已接受和拟议中的重要架构决策。

## 设计纪律

只有必须在独立实现间保持一致的概念，才能进入 Flower Specification。语言习惯、运行时优化、SDK 便利 API 和供应商集成都应留在实现或适配层。

新增设计前应回答：

1. 这是跨实现语义，还是某个实现的工程选择？
2. 能否由一致性测试观察并验证？
3. 是否把 Flower 推向了另一门通用编程语言？
4. 如果 Rust 或 WASM 被替换，这个概念是否仍然成立？

相关决策发生变化时，先新增或替代 ADR，不要静默改写历史决定。
