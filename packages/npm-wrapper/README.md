# @eastream/loong

Thin npm distribution channel for the `loong` CLI.

## What it does

- downloads the matching prebuilt `loong` binary from GitHub Releases
- caches it under the package directory
- exposes the `loong` command through npm's normal `bin` wiring
- prefers the newer human-readable release asset names and falls back to legacy rust target-triple names when older releases still use them

## Install

```bash
npm install -g @eastream/loong
```

## Notes

- this package is only a distribution wrapper
- this is not a JavaScript implementation of loong
- the Rust CLI remains the source of truth
- release binaries are still published through GitHub Releases
