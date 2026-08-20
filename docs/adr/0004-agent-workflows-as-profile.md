# ADR-0004：Agent 编排作为可组合 Profile

- Status: Proposed
- Date: 2026-08-21

## Context

Flower 的一个重要目标场景是软件工程 Agent 编排：Planner 产出计划，人工批准后动态创建多个 Executor，Reviewer 给出结构化 verdict，必要时返工、追加执行者或升级人工决策。节点还需要按策略读取代码、任务状态、Artifact 和长期记忆。

如果把 Planner、Executor、Reviewer 或某个 memory 产品直接做成核心 Node 类型，通用 Workflow 语义会被当前 AI 工具和供应商绑死。反过来，如果核心只有无法约束的通用字符串节点，又无法表达审查返工、上下文权限和结构化输出。

## Proposed decision

Agent 编排作为 Flower 上层 profile：

- 通用核心保留 Node、Event、Effect、Artifact、Capability 和状态转换；
- profile 增加 Actor、ContextPolicy、CapabilityPolicy 与 OutputContract；
- Actor 可以是 Human、Agent、Tool、Policy 或 Composite；
- Planner 只产生结构化 `ExecutorSpec[]`，由 Runtime 在批准后执行 fan-out；
- Review 产生 verdict，不等同于 Finish；
- `approve` 进入 Acceptance/Finalize，`request-changes` 回到 Rework/Execute；
- memory 通过可替换 ContextProvider 接入，读写权限独立，长期写入受 commit policy 管理；
- Agent 供应商、CLI、模型和 memory backend 都留在 adapter 层。

## Consequences

正面影响：

- 通用 Runtime 不依赖 Codex、Claude、ai-memory 或其他具体产品；
- 同一 Workflow 可以在 Human 与 Agent Actor 之间替换；
- Review、返工和验收成为显式控制流；
- 动态 fan-out 由经过批准的运行时 Effect 完成，模型不会直接获得进程控制权；
- 上下文和能力可以按 Node 审计。

代价与待决问题：

- 需要定义 profile 与核心 schema 的组合方式；
- `ExecutorSpec`、verdict 与 Artifact contract 尚未冻结；
- message injection、进度流和长期记忆提交仍需独立语义；
- 必须用非 Agent 工作流验证核心没有被 profile 污染。

## Alternatives

### Agent 类型进入 Flower Core

不采纳。Agent 是一种 Actor/Backend，不是所有工作流实现都必须理解的基础语义。

### memory 作为固定 Workflow Node

不采纳。memory 是上下文来源和提交目标，强制建模为业务步骤会扭曲数据访问与权限边界。

### Review 成功后直接 Finish

不采纳。实现审查、验收、收尾和整个 Workflow 完成是不同事件；大部分失败 verdict 还必须进入返工路径。
