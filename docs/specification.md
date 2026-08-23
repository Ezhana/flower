# Flower 规范组织

Flower Specification 由三个缺一不可的部分构成。仅有 schema、文档或测试中的任意一个，都不足以定义兼容实现。

## 1. Machine-readable model

机器模型描述能够无歧义结构化的内容：

- 值与接口类型；
- Workflow、Node 与 Edge；
- 状态和允许的转换；
- 错误分类与属性；
- Capability 与执行策略；
- 版本和兼容性元数据。

它必须可解析、可验证、可版本化，并能生成 schema、binding 和测试 fixture。模型应保持声明式、有限且非图灵完备。

YAML 只是后续候选输入格式。Compiler 输出的 `ExecutableWorkflowPlan` 才能被 Kernel、Component 和 Host 使用。

## 2. Normative semantics

并发、持久化、崩溃恢复、顺序、幂等、重试和重放等行为无法仅靠结构 schema 完整表达，必须由规范性文本定义。

规范使用 RFC 2119 风格术语：

- **MUST / MUST NOT**：兼容实现的硬性要求；
- **SHOULD / SHOULD NOT**：除非有明确、可说明的原因，否则应遵守；
- **MAY**：允许但不要求的行为。

初始不变量包括：

1. Execution **MUST** 拥有稳定标识；
2. 重试 **MUST** 创建新 Attempt，不得覆盖之前的尝试；
3. Runtime **MUST NOT** 自动重试 terminal error；
4. Host **MAY** 拒绝不支持或未授权的 Capability；
5. Runtime 将 Execution 标记为 `Completed` 前，结果 **MUST** 满足规范要求的持久化保证；
6. 外部副作用的请求与结果 **MUST** 可关联，重放 **MUST NOT** 默认重复执行已确认的副作用；
7. Binding **MUST NOT** 改变公共 Value、Error 或 State 的含义。

v0.1 已冻结线性成功路径；失败、Attempt、重试与取消条款仍待后续版本补全。

## 3. Conformance suite

一致性测试把可观察语义变成所有实现共享的可执行例子。测试使用与语言无关的输入、事件序列和预期输出，例如：

```text
Given  max-attempts = 3
And    the node always returns retryable Timeout
When   the execution starts
Then   three distinct attempts are recorded
And    the execution ends as Failed
```

测试至少覆盖：

- 不透明 Payload 编解码与边界值；
- 合法和非法状态转换；
- 重试、取消和超时；
- Effect 去重与恢复；
- Capability 拒绝；
- Binding round-trip；
- 版本兼容与迁移。

Rust、Go、.NET 或其他独立 Runtime 只有通过同一套 suite，才能声称实现同一版 Flower Specification。

## 版本分离

项目至少维护三种独立版本：

| 版本 | 管理对象 |
| --- | --- |
| Specification version | 公共数据与行为语义 |
| Binding version | WIT、C ABI、RPC 等映射 |
| Implementation version | Rust Runtime、SDK 和工具发布 |

三者不能共用一个兼容性承诺。Binding 升级不一定改变规范；实现发布也不一定改变 Binding。

## 变更规则

规范变更必须同时回答：

1. 变更属于结构、规范性行为还是实现细节？
2. 哪个 conformance case 能观察到变化？
3. 是否破坏旧 Workflow、Node 或 Binding？
4. 是否需要新 ADR、迁移规则或 major version？

无法给出可观察行为的内容通常不应进入核心规范。
