# Flower 路线图

路线图只围绕纯解释器与调用接口展开。

## 当前范围

- 单一 `flower-engine` Rust 实现；
- `compile` 与 `transition` 两个公共逻辑入口；
- 无 WASI import 的 WebAssembly Component；
- Go SDK 调用同一 Component；
- 线性 Activity 执行、Attempt、失败和确定性 retry/timer；
- 原生 Rust 与 Component 共享回归 fixtures。

## 下一阶段

1. 收紧 WIT 字段与错误分类，删除无法证明用途的字段；
2. 提供完整 Rust 与 Go SDK 调用示例；
3. 明确 Plan 和 Snapshot 的序列化升级策略；
4. 评估 gateway 前先定义最小可观察路由输入，不引入通用表达式语言；
5. 增加 fuzz/property tests，覆盖伪造 Plan、Snapshot 和身份链。

## 明确不做

- CLI；
- 内置数据库或 Store trait；
- Worker、scheduler 或 timer service；
- 通用工作流语义规范；
- 多语言独立状态机实现；
- 为假想 Agent、Human、Tool 产品预埋核心类型。

只有出现具体调用方和可验证需求后，才扩展 WIT 或解释器行为。
