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

Loong is a Rust foundation for vertical AI agents.
Its priorities are security, performance, extensibility, and long-term evolution.
Agents should work reliably in real environments.
They should collaborate with developers beyond a single demonstration.

## Why Loong

### Security is more than a sandbox

Sensitive operations should be authorized before they happen.
Loong aims to make permission decisions part of its normal execution path.
It should not rely on scattered checks or sandbox-only security.
Sandboxes can protect untrusted extensions; they cannot replace authorization.

### Performance starts with the foundation

Loong uses Rust.
Core execution paths should stay fast.
Runtime overhead should remain predictable.
Concrete performance claims require representative workload benchmarks.
Loong aims to minimize its own runtime overhead.
Today's high hardware costs increase its importance for edge and commodity hardware.

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
The [release guide](docs/development/releasing.md) documents the crates.io
workflow for `loac` and `loac-macros`.

The current checks are:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
```

</details>

## License

Loong is licensed under the [MIT License](LICENSE-MIT).
