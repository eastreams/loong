# 🐉 Loong - Rust Base for Vertical AI Agents

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="./assets/logo/loong-logo-dark.png" />
    <source media="(prefers-color-scheme: light)" srcset="./assets/logo/loong-logo-light.png" />
    <img src="./assets/logo/loong-logo-light.png" alt="Loong" width="280" />
  </picture>
</p>
<p align="center"><strong><em>"Originated from the East, here to benefit the world"</em></strong></p>

<p align="center">
  <a href="README.md">English</a> |
  <a href="README.zh-CN.md">简体中文</a>
</p>

> [!IMPORTANT]
> This repository is an independent-history rewrite of Loong. It currently
> contains an architecture and contract skeleton, not a usable agent product.

Loong is a Rust base for vertical AI agents, designed to be secure,
high-performance, extensible, and able to evolve over the long term. Its goal
is to help AI agents work reliably in real environments, not only complete a
single demonstration.

## Why Loong

### Security is more than a sandbox

Sensitive operations should be authorized before they happen. Loong is designed
to make permission decisions part of the normal execution path, instead of
depending on checks scattered throughout the application or treating a sandbox
as the entire security model.

A sandbox can still provide extra protection for extensions you do not trust.
It complements authorization rather than replacing it.

### Performance starts with the foundation

Loong uses Rust for its native foundation, with the goal of keeping core work
fast and overhead predictable. Concrete performance claims will be backed by
benchmarks once representative workloads exist.

### Extend it at your own pace

Loong is intended to support two ways to extend the product:

- Build deep or performance-sensitive integrations in Rust and compile them
  with Loong.
- Load scripts and similar extensions at runtime for faster iteration without
  rebuilding the host application.

The same security expectations apply to both. The exact runtime extension
system is still being designed.

### Built to keep evolving

The rewrite deliberately begins with a small foundation. New product areas will
be added as their responsibilities and safety boundaries become clear, keeping
Loong understandable as it grows.

## Current Status

This rewrite is at an early foundation stage. It is not ready to install or use
as a complete agent product yet. The repository will describe capabilities as
they become real rather than presenting planned work as finished.

<details>
<summary>Developer information</summary>
The technical design is documented in [ARCHITECTURE.md](ARCHITECTURE.md), and
unresolved design decisions are tracked in
[docs/open-questions.md](docs/open-questions.md).

The current checks are:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
```

</details>

## License

Loong is licensed under the [MIT License](LICENSE-MIT).
