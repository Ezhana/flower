# WIT 接口边界

`wits/engine.wit` 是 Flower 的跨语言接口定义。它保证调用双方对 record、variant、list、option 和 result 的布局达成一致，但不宣称定义一个可由多套实现独立满足的通用工作流规范。

## 导出函数

```text
compile(definition) -> result<plan, diagnostics>
transition(plan, optional snapshot, event) -> result<transition, engine-error>
```

Rust `flower-engine` 是这两个函数行为的唯一事实来源。Go SDK 加载 Rust Component；它不会重新实现 compile 或 transition。

## 稳定性

WIT package 自己携带接口版本。Plan 不再包含名为 `specification-version` 的重复字段，也不存在独立的“Flower Specification version”。

接口变更必须同步完成：

1. 更新 WIT；
2. 更新 Rust Component 转换；
3. 重新生成 Go ABI binding；
4. 更新公开 SDK 类型；
5. 重新生成共享 fixtures；
6. 通过 `just verify`。

Fixtures 是同一 Rust 实现在原生调用与 Component 调用之间的回归数据，不是第三方实现认证套件。

## 第三方实现

第三方可以实现相同 WIT，但只能声称“ABI 形状兼容”。除非它逐项复现 Rust 实现的可观察行为，否则不能声称与 Flower Engine 行为兼容。
