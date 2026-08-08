# Loong Agent 指南

1. 架构性内容需要简洁注释，让后来者理解边界为何存在以及禁止哪些依赖。
2. 任何能用类型表达的安全性，不要依赖随处验证。例如使用外部不可构造的 `Granted<Action>`，而不是 `Grant { token, action }`。
3. 对于新增的裸 helper，必须能解释它的存在理由和作用；不要堆积只转发参数或转换等价结构的 helper。
4. 库错误使用 `thiserror`；应用入口确实需要聚合上下文时使用 `anyhow`。
5. 当修改导致出现大文件，考虑拆分，尤其是 mod tests 一旦变大就该拆出。
6. Rust module 使用 `{module}.rs` 命名。子模块放在同名目录中。避免使用
   `mod.rs` 作为 parent module；没有子模块的 helper 保持为单个文件。
