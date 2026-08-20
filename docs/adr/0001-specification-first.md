# ADR-0001：规范优先，语义独立于实现

- Status: Accepted
- Date: 2026-08-21

## Context

如果先把 Rust public API 当成产品定义，再生成 C#、Go、Python、WIT 或 RPC 适配层，Rust 的 trait、泛型、生命周期、async 模型和内存所有权会泄漏到所有语言。最终所谓的“跨语言规范”只会是 Rust API 的一组有损翻译。

反过来，只定义一个 YAML schema 也不够。持久化、重试、并发、恢复、幂等和重放包含 schema 无法完整表达的行为语义。

## Decision

Flower 采用 specification-first：

1. machine-readable model 定义结构；
2. normative semantic specification 定义行为；
3. conformance suite 验证可观察语义；
4. Rust Runtime、WIT、C ABI 和 RPC 都是上述规范的实现或 binding。

公共语义不能只存在于某个实现的类型或测试中。只有必须在独立实现间一致的概念才能进入 Flower Specification。

## Consequences

正面影响：

- 可以独立实现 Go、.NET 或其他 Runtime；
- 可以替换 binding 技术而不重新定义 Flower；
- 兼容性由测试和版本规则判断，而不是由“能编译”判断；
- 公共模型不会被单一语言的便利特性绑死。

代价与约束：

- 开发一个功能时通常要同步维护模型、规范文本和测试；
- Rust 实现不能先行冻结公共 API，再要求规范追认；
- 早期迭代速度会慢于只写单语言库，但能避免后期多语言重构成本。

## Rejected alternatives

### Rust API 作为唯一事实来源

拒绝。它无法满足实现独立性，并会迫使其他 binding 模拟 Rust 特有语义。

### WIT 作为唯一事实来源

拒绝。WIT 擅长表达 Component 接口类型，但不定义 durable execution、事务、重放和错误策略的全部行为。

### YAML schema 作为完整规范

拒绝。为了表达所有行为而不断增加条件、表达式和时序逻辑，会把 Flower 变成一门劣化的通用编程语言。
