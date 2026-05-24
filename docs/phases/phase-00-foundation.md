# Phase 0 — Foundation & architecture

> First session (S0.1) deliverable. Freezes the scope and decisions for the
> foundation before any code is written. See [`../ROADMAP.md`](../ROADMAP.md) and
> [`../METHODOLOGY.md`](../METHODOLOGY.md).

## Goal

Build a robust, well-separated base for SysTUI: a Rust workspace with cleanly
separated crates, the core contracts (transport / collector / action), the security
model plumbing, and a stable empty TUI that loads config and runs *mocked* collectors
without crashing. Phase 0 ships no public release; it is the substrate that `v0.1`
(Phase 1) builds on, and lives on the `release/v0.1` branch.

This phase exists because, per `Product.md` §2 and §13, getting transport abstraction
and layer separation right *first* is what makes SSH (Phase 5) and every later module
a simple addition instead of an architecture rewrite.

## In scope

- Cargo **workspace** with all target crates (even if some are near-empty stubs):
  `systui-cli`, `systui-core`, `systui-ui`, `systui-transport`, `systui-collectors`,
  `systui-actions`, `systui-security`, `systui-report`, `systui-storage`, `systui-testkit`.
- `systui-core`: typed errors (`thiserror`), `CommandSpec`, `CommandOutput`, base
  domain models, config schema, `Collector` and `Action` traits, execution modes
  (read-only / safe / privileged).
- `systui-transport`: `Transport` trait + `LocalTransport` + `MockTransport`.
- `systui-cli`: `clap` CLI with the subcommand skeleton (`systui`, `systui ssh`,
  `systui fleet`, `systui report`, `--read-only`), config load/merge, `tracing` setup.
- `systui-ui`: `ratatui`/`crossterm` event loop, global app state, tab navigation,
  theme, key bindings, empty dashboard, and the core UI states (loading / empty /
  error / permission-denied).
- `systui-testkit`: fixtures + mock helpers; basic unit tests.
- CI skeleton + `rustfmt`/`clippy` configuration.

## Out of scope (deferred)

- Real system collectors (CPU/RAM/disk/etc.) → Phase 1.
- Real actions execution, audit log, read-only enforcement details → Phase 2.
- Network, security findings, docker, crons, SSH, reports, DB, fleet, policies →
  later phases per the roadmap.

## Key decisions

- **No free-form command strings.** All execution goes through
  `CommandSpec { program, args, requires_privilege, .. }` to avoid injection and
  SSH quoting issues (`Product.md` §6 Fase 0).
- **Transport is the seam.** Every collector/action talks to a `Transport`, never to
  the OS directly, so Local/SSH/Mock are interchangeable (`Product.md` §2).
- **UI requests, engine decides.** The UI never runs `systemctl`; it emits action
  requests handled by the action engine (`Product.md` §10).
- **Async runtime: `tokio`.** Transport trait methods are `async` (`Product.md` §2).
- **Security model from day 0.** Execution modes and the read-only flag are modeled in
  core now, even though enforcement lands in Phase 2.
- **Error handling:** `thiserror` for library crates, `anyhow` at the CLI boundary.
- **MSRV / edition:** Rust **2024 edition**, MSRV `1.85` (toolchain in use is 1.95;
  edition 2024 is stable, so we adopt it now to avoid a later migration).

## Sessions

- **S0.1 — Context** *(this file)*.
- **S0.2 — Workspace scaffold**: workspace `Cargo.toml`, all crates, shared deps,
  `rustfmt.toml`, clippy lints, CI skeleton.
- **S0.3 — Core contracts** *(done)*: errors, `CommandSpec`/`CommandOutput`, domain
  models, config schema, exec modes, and the `Transport`/`Collector`/`Action` traits.
  The `Transport` trait lives in `systui-core` as a contract (not in
  `systui-transport`) because `Collector`/`Action` reference it; this avoids a
  dependency cycle.
- **S0.4 — Transport implementations** *(done)*: `LocalTransport` and
  `MockTransport` in `systui-transport` (the trait already lives in core).
- **S0.5 — CLI & config** *(done)*: `clap` subcommand skeleton (`local`/`ssh`/
  `fleet`/`report`, global `--read-only`/`--config`), config loading in
  `systui-storage` (defaults when absent), `tracing` to stderr via `SYSTUI_LOG`,
  execution-mode resolution.
- **S0.6 — TUI shell** *(done)*: ratatui/crossterm event loop, `App` state, tab
  navigation, dark theme, key bindings, help overlay, empty dashboard and UI
  states (loading/empty/partial/permission-denied/error). Rendering is a pure
  function tested headlessly with `TestBackend`. `systui` (local) launches it.
- **S0.7 — Wire-up & tests** *(done)*: `HostInfoCollector` (reads hostname/kernel
  via `uname`) rendered end-to-end through transport → collector → state → UI. The
  sync UI loop drives async collectors via a tokio runtime (`block_on`), with `r`
  to refresh. Headless tests prove the full path with `MockTransport`.

**Phase 0 complete.** All sessions done; the foundation now feeds v0.1 (phase 1) on
the same `release/v0.1` branch.

## Definition of Done

- `systui` launches a stable TUI (open, navigate tabs, quit cleanly).
- Config loads from the documented path with sane defaults.
- A mocked collector's data renders in the UI through the full
  transport → collector → state → UI path.
- A failing/erroring module shows an error state without crashing the app.
- `cargo fmt --check`, `cargo clippy -D warnings`, and `cargo test` all pass.

## Risks & open questions

- **SSH crate choice (Phase 5)** is deferred, but `Transport` must not leak any
  Local-only assumptions now. Mitigation: keep the trait minimal and IO-agnostic.
- **Privilege escalation strategy** (sudo vs polkit vs run-as) — decide in Phase 2;
  Phase 0 only models `requires_privilege`.
- **Config path layout** (`~/.config/systui/`, `~/.local/share/`, `~/.cache/`) per
  `Product.md` §11 — confirm with `directories`/`dirs` crate during S0.5.
- **ratatui/crossterm versions** — pin in S0.2 and keep consistent across crates.
