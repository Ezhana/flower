# Flower 路线图

路线图按可验证的退出条件组织，不承诺日期。任何阶段都不能仅以“创建了目录”或“定义了接口”视为完成。

## Phase 0：统一语言与决策记录

目标：建立单一项目命名、核心词汇、系统边界和 ADR 流程。

退出条件：

- README、架构、核心概念和 ADR 互相链接且不矛盾；
- 已实现与拟议能力有明确标记；
- 不再使用一次性计划稿充当架构事实来源。

## Phase 1：最小语义核心

目标：冻结第一版 Value、Node、Execution、Attempt、Error、Event、Effect 和 Capability 语义。

交付物：

- machine-readable schema；
- 规范性状态转换与错误规则；
- 与语言无关的 conformance fixtures；
- 至少覆盖正常完成、可重试失败、终止失败、取消和能力拒绝。

退出条件：独立实现者不阅读 Rust 源码，也能根据规范通过核心测试。

## Phase 2：Rust 参考实现（进行中）

目标：用 Rust 验证语义，而不是让 Rust API 反向定义规范。

交付物：

- `flower-ir`：规范化、无运行时依赖的 IR；
- `flower-compiler`：解析、验证和诊断；
- `flower-runtime`：原生节点、状态转换和 Effect dispatch；
- `flower-cli`：validate、run 和 inspect 的最小闭环。

退出条件：示例 Workflow 能通过 CLI 执行，并通过 Phase 1 的全部 conformance cases。

## Phase 3：WIT / WebAssembly Component binding（执行内核纵向切片已完成）

目标：验证跨语言扩展节点，并为外部 Host 提供不含 I/O 的确定性执行内核；不把完整 Runtime 搬进 WASM。

交付物：

- 版本化的 WIT package 与最小 world；
- Wasmtime Host、资源限制和能力连接；
- Rust guest SDK；
- 至少一个非 Rust guest，用于证明接口不是 Rust 类型的翻版；
- component-level conformance cases。

退出条件：同一 Workflow 可以替换两个不同语言实现的等价 Node，得到相同的可观察语义。

## Phase 4：Native binding

目标：让非 Rust 宿主应用在同进程中调用 Runtime。

交付物：

- 小型、版本化 C ABI；
- 明确的内存所有权、错误和 panic 边界；
- 至少两个语言 binding 的 round-trip 测试。

退出条件：C ABI 不暴露 Rust 布局，跨平台打包策略已被实际验证。

## Phase 5：进程外 Runtime

目标：支持无法或不应嵌入原生库的客户端和 Worker。

交付物：

- `flowerd`；
- 版本化 HTTP 或 RPC contract；
- durable storage 与恢复；
- 远程取消、事件订阅和 Artifact 访问；
- 鉴权、幂等和观测规则。

退出条件：Runtime 进程重启后能恢复长时间运行的 Workflow，且不会重复已确认副作用。

## 后续候选

以下内容只有在前述语义稳定并有实际需求后再进入路线图：

- Agent Workflow Profile 与多 Agent 动态 fan-out；
- Human-in-the-loop UI；
- 可替换 ContextProvider 与长期记忆；
- container / remote Worker backend；
- WASI 0.3 async、stream 和 future 的 binding；
- workflow migration 与多版本并行运行。

这些方向不能绕过语义核心直接堆叠实现，否则只会得到一组彼此不兼容的适配器。
