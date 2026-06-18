# 代码规范

本文档定义 Loong 源码变更必须遵守的代码规范。英文版和简体中文版必须描述同一组规则。

## 范围

除非某条规则明确缩小适用范围，否则这些规则适用于仓库中的新增和修改代码。

Vendored 第三方源码只有位于明确的 vendored 路径下才豁免。

本文档不替代 `AGENTS.md`、`CLAUDE.md`、`CONTRIBUTING.md`、
`docs/design-docs/core-beliefs.md` 或 `docs/design-docs/layered-kernel-design.md`。

## 规则术语

- `MUST` 表示必须遵守。
- `MUST NOT` 表示禁止使用。

## Rust 代码形态

### RUST-1：公共 API 保持增量

公共 API 必须保持增量演进，除非变更链接到已文档化的破坏性变更决策。

### RUST-2：分层边界是强制规则

低层 crate 禁止包含领域特定行为，除非相关架构文档明确允许该依赖方向。

### RUST-3：新增依赖必须说明理由

新增 workspace 依赖必须在 PR 中说明依赖名称、所属 crate，以及为什么现有 workspace
依赖或标准库不足以解决问题。

### RUST-4：禁止的 Rust 模式仍然禁止

Rust 代码禁止使用 `unwrap`、`expect`、`panic`、`todo`、`unimplemented`、unsafe 代码、
stdout 调试输出或 stderr 调试输出。

## 函数大小

### SIZE-1：函数默认最多 50 行

每个 Rust 函数和方法默认最多 50 个物理源码行，除非函数本身标注
`#[allow(clippy::too_many_lines)]`。

行数从 `fn` 签名所在行开始，到匹配的右花括号所在行结束。签名前的属性和文档注释不计入。
函数内部的空行计入。

### SIZE-2：超长函数例外必须向 reviewer 说明原因

任何超过 50 个计数行的函数或方法，必须在该函数或方法上标注
`#[allow(clippy::too_many_lines)]`。

作者必须在 PR、review 讨论或相邻源码注释中向 reviewer 说明例外原因。

### SIZE-3：超长函数必须抽取

当函数会超过 50 个计数行时，校验、转换、执行、格式化或 setup 工作必须抽取到更小的具名函数，
除非该函数符合 `SIZE-2`。

## 测试组织

### TEST-1：私有行为测试放在模块旁边

测试私有 helper 或私有模块行为的测试，必须放在实现所在的同一个 Rust 源文件中。

### TEST-2：公共行为测试使用集成测试 surface

测试公共 CLI、runtime、protocol 或跨 crate 行为的测试，必须放在所属 crate 的 `tests/`
目录或 workspace 的 `tests/` 目录下。

### TEST-3：单元测试专用 helper 使用 test_utils.rs

仅由内嵌测试模块或单元测试模块使用、且生产代码不使用的函数，必须放在 `test_utils.rs`
文件中。

`test_utils.rs` 的模块声明必须由 `#[cfg(test)]` guard 保护。生产代码禁止导入
`test_utils.rs`。

### TEST-4：集成测试支持代码使用 test_support.rs

必须编入 crate 以供集成测试使用的 helper，包括 mock provider、fake transport、harness
builder 和 integration fixture，必须放在 `test_support.rs` 中。

### TEST-5：测试模块名称受限

Rust 测试模块必须命名为 `tests`，或以 `tests_` 开头。

### TEST-6：测试模块必须带测试 guard

Rust 测试模块必须由 `#[cfg(test)]` guard 保护。

### TEST-7：内嵌测试模块必须位于文件末尾

内嵌 Rust 测试模块必须出现在源文件末尾。

### TEST-8：内嵌测试模块后禁止生产代码

生产代码禁止出现在内嵌 Rust 测试模块之后。

### TEST-9：测试禁止使用真实用户状态

测试禁止读取或写入开发者真实 home 目录。需要 Loong 状态的测试必须将 `LOONG_HOME` 设置为
隔离的临时目录，或使用 `./scripts/cargo-local-toolchain.sh test`，该脚本会提供隔离的默认测试
home。

### TEST-10：真实网络测试必须隔离

测试禁止执行真实网络调用，除非该测试被显式标记为 ignored，或被默认关闭的 feature gate 隔离。

## 测试命名

### NAME-1：测试函数使用行为名称

Rust 测试函数名必须用 `snake_case` 描述被保护的行为。

### NAME-2：禁止模糊测试名称

测试函数名禁止为 `test_basic`、`test_success`、`test_error`、`test_failure`、
`test_regression`，也禁止使用这些名称加数字后缀的形式。

### NAME-3：Issue ID 不能替代行为名称

Bug ID、issue ID 和 incident ID 禁止作为测试名称中唯一的行为描述。需要记录 ID 时，将其写入注释。

## Fixtures 和 Snapshots

### DATA-1：Fixture 放在所属测试旁边

Fixture 必须放在拥有它的测试文件旁边的 `fixtures/` 目录下。共享 fixture 必须放在共享
`fixtures/` 目录下，并带有 README 说明所属测试套件。

### DATA-2：Fixture 名称描述场景

Fixture 文件名必须包含被测试的领域或行为。Fixture 文件名禁止只使用 `sample`、`test`、
`temp`、`tmp`、`data` 或纯数字名称。

## 错误处理和可观测性

### ERR-1：安全关键结果必须可观测

Policy denial、capability failure、token lifecycle change 和 plane invocation 必须产生 kernel
和 reliability 文档要求的 audit evidence。

### ERR-2：安全关键 Result 必须处理

来自 policy、capability、audit 和 security-scan 操作的 Result 必须被显式处理。对这些操作禁止使用
`let _ = ...`。

### ERR-3：泛化错误字符串不能隐藏受治理失败

新增或修改的 governed runtime 代码禁止将 policy denial、capability failure 或 audit-write
failure 折叠为泛化错误字符串。
