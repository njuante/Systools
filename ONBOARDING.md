# SysTUI — Onboarding

Read this first if you (human or AI assistant) are starting a new working session
on SysTUI. It captures the non-obvious rules so you can be productive immediately.

## What this is

**SysTUI** is a fast, agentless **TUI in Rust** for Linux server administration.
It is not just a metrics monitor — its core loop is
**Detect → Explain → Correlate → Act safely → Report**: it correlates a failed
service to its logs, process, ports, risk and a safe remediation.

Full design and module specs live in [`Product.md`](Product.md) (the only Spanish
document; everything else is English).

## Read these, in order

1. [`Product.md`](Product.md) — product vision, architecture, per-module specs.
2. [`docs/ROADMAP.md`](docs/ROADMAP.md) — phases → versions → sessions, up to v1.0.
3. [`docs/METHODOLOGY.md`](docs/METHODOLOGY.md) — how we work (branches, sessions, gates).
4. [`docs/phases/`](docs/phases/) — the context file for the **active** phase, first.

## Non-negotiable rules

- **Commits never mention Claude, AI, or any assistant.** No co-author trailers.
- **Conventional Commits**, in English (`type(scope): subject`). Everything —
  code, identifiers, app UI text, docs, commits — is English. `Product.md` is the
  only exception.
- **Branching:** `main` holds only stable, tagged releases. Each version is built
  on `release/vX.Y`; when its Definition of Done is met, merge `--no-ff` into `main`
  and tag `vX.Y`.
- **First session of every phase** writes that phase's context file in
  `docs/phases/` *before* any code.
- **Quality gates before every commit** must be green:
  ```sh
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  ```

## Architecture guardrails

- The **UI only requests actions**; the action engine (`systui-actions`) decides
  and runs them. The UI never executes commands.
- **No free-form command strings.** Use `systui_core::CommandSpec { program, args,
  requires_privilege, timeout }`. This kills shell-injection and SSH-quoting bugs.
- Every collector/action runs through a `systui_core::Transport`
  (`LocalTransport` / `SshTransport` / `MockTransport`) — never the OS directly.
- The **security model** (read-only / safe / privileged modes, confirmations,
  backups, audit) exists from phase 0, not bolted on later. Default mode is
  read-only (`ExecutionMode::default()`).
- Missing permissions **degrade gracefully** ("partial data"), never crash.
- Every command-output **parser is covered by fixture/golden-file tests**.

## Repository layout

```
Product.md                 # design spec (Spanish)
ONBOARDING.md              # this file
docs/ROADMAP.md            # phase/version/session plan
docs/METHODOLOGY.md        # working method
docs/phases/               # per-phase context files
Cargo.toml                 # workspace: shared deps + lints
rustfmt.toml               # edition 2024, width 100
.github/workflows/ci.yml   # fmt + clippy + test
crates/
  systui-core/             # models, errors, config, contracts (Transport/Collector/Action)
  systui-transport/        # LocalTransport, MockTransport (SSH in phase 5)
  systui-cli/              # `systui` binary entry point
  systui-ui/               # ratatui shell, navigation, state
  systui-collectors/       # system/process/service/log/network readers
  systui-actions/          # action engine (phase 2)
  systui-security/         # findings & posture checks (phase 3)
  systui-report/           # JSON/Markdown/HTML reports (phase 6)
  systui-storage/          # config, profiles, cache, audit log
  systui-testkit/          # fixtures, golden files, mock helpers
```

## Build & run

```sh
cargo build --workspace          # build everything
cargo run --bin systui           # run the binary (scaffold today)
cargo test --workspace           # run all tests
```

The toolchain in use is Rust 1.95 (edition 2024, MSRV 1.85). The workspace builds
offline once dependencies are cached.

## Current state

Source of truth is `git log` and the active phase context file — check them, this
snapshot may lag.

- Branch: `release/v0.1`.
- **Phase 0 (Foundation)** in progress. Done: S0.1 context, S0.2 workspace scaffold,
  S0.3 `systui-core` contracts, S0.4 `LocalTransport`/`MockTransport`, S0.5 CLI
  skeleton + config loading + tracing.
- **Next: S0.6** — TUI shell (ratatui event loop, app state, navigation, theme,
  empty dashboard, UI states), then S0.7 wire-up. After phase 0 the foundation
  feeds v0.1 (phase 1).

## Starting a session

Confirm the session plan before writing code, then: implement → run the three
quality gates → commit (Conventional Commits, no assistant mention) → update the
phase context file's session status.
