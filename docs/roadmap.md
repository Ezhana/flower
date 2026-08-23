# Flower 路线图

## 已完成：v0.1 语义内核

- 定义与不可变执行计划分离；
- 稳定 fingerprint、execution/revision/event/effect 身份；
- 纯确定性 transition 与结构化错误；
- 共享 Rust/Go conformance fixtures；
- 无 WASI Component 与生成的 Go ABI；
- 原子 snapshot/event/outbox 内存 Store、replay 与并发冲突测试。

## 下一阶段：失败与 Attempt

严格按以下顺序推进：

1. `NodeExecutionFailed`；
2. `AttemptId` 与不可覆盖的 attempt history；
3. terminal/retryable error；
4. retry policy；
5. cancellation；
6. timeout；
7. capability rejection；
8. YAML frontend；
9. CLI validate/run/inspect；
10. gateway；
11. edge condition。

Gateway 之前必须先冻结失败、Attempt、replay 和重试语义。PostgreSQL 必须等内存 Store 的崩溃恢复、重复投递和并发冲突行为稳定后再引入。
