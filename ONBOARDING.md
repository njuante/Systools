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
2. [`docs/ROADMAP.md`](docs/ROADMAP.md) — phases → versions → sessions, v0.1 → v1.0 (all shipped).
3. [`docs/METHODOLOGY.md`](docs/METHODOLOGY.md) — how we work (branches, gates, commits).
4. [`CHANGELOG.md`](CHANGELOG.md) — what landed in each version (better than the history below).
5. [`docs/BACKLOG.md`](docs/BACKLOG.md) — the post-v1.0 candidates, **including the three tasks below**.

## Where the project is now: v1.0.0 shipped

**The v0.1 → v1.0 roadmap is complete.** `v1.0.0` is tagged on `main` and published as a
GitHub Release (static musl binaries, `.deb`/`.rpm`, AUR, `install.sh`, checksums, SBOM,
keyless cosign signature). v1.0 added per-distro integration tests, parser fuzzing, a
large-log benchmark, a security review and packaging — see `CHANGELOG.md` and
`docs/phases/phase-10-v1.0-release.md`. A security/quality audit
([`docs/AUDIT-2026-05.md`](docs/AUDIT-2026-05.md)) found no vulnerabilities.

New work is now **v1.1+** (`docs/ROADMAP.md` "Out of scope until v1.1+").

## What to build next (the three tasks)

These are the three implementations to do well in a fresh session. Full notes in
[`docs/BACKLOG.md`](docs/BACKLOG.md); concrete pointers here. The approved UI prototype is
in [`docs/interfaz/`](docs/interfaz/) (Ratatui spec + screenshots under
`_extracted/screenshots/`) — the first two tasks bring two tabs up to that prototype.

**Read the prototype screenshots first**, then compare against the live tabs.

### 1 & 2 — System and Processes tab UI parity

Both tabs render far sparser than the prototype. **The data is already collected** — this
is UI wiring, exactly like the v0.8.2 reskin / v0.8.4 data-parity work (match the
prototype layout, **real data only, never mock**, omit panels with no real data).

- **System tab** — today a single plain-text block: `systui-ui/src/ui.rs::system_text`
  (dispatched at the `(Ready, Some(snap), Tab::System)` arm of `render`, ~line 376).
  Rebuild as a multi-panel screen (hardware/identity, CPU/RAM/swap gauges, disks table,
  load, logged-in users). All fields already exist on `SystemSnapshot` (`os`, `kernel`,
  `uptime_secs`, `load`, `cpu`, `memory`, `swap`, `disks`, `users`).
- **Processes tab** — today a flat top-20 table with **no detail panel**:
  `ui.rs::render_processes` (~line 859). Add a **detail side panel** and optionally a
  **tree** view, and scroll past 20. Model the layout on the existing
  `render_service_detail` / `render_container_detail` / `render_database_detail`.
  - **Non-obvious bit:** those detail panels render from data already in `App`. The
    process detail (cmd/cwd/open files/ports) is **not** in `App` — it comes from a
    separate async call `systui_collectors::process_detail(transport, pid)`, and the
    tree from `build_process_tree`/`TreeRow`. So you must wire a fetch for the selected
    PID (on selection, or fold it into the gather), going through the v0.8.3 background
    refresh — don't block the UI thread.

### 3 — Optional "expert console" (free-form shell), gated

A tab to type raw commands. **This deliberately steps outside the core guarantee** (no
free-form commands; everything via `CommandSpec` + the action engine). Only build it with
the guardrails in `docs/BACKLOG.md`: **off by default**, **disabled in read-only mode**,
**every command audited**, a clear "leaving the safe path" boundary, and a master
password as an *access* gate only (store an argon2 **hash**, never plaintext — it is not
the security mechanism). Keep it clearly separate from engine-mediated actions.

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
cargo run --bin systui           # run the TUI locally (Hosts grid)
cargo run --bin systui -- local  # inspect the local machine
cargo test --workspace           # run all tests (≈360)
```

Edition 2024, MSRV 1.85 (`rust-version` in `Cargo.toml`). Builds offline once deps are
cached. Per-distro integration tests are feature-gated (`-p systui-collectors --features
integration`) and run in CI, not in the default workspace test run.

## UI implementation notes (for the three tasks)

The next work is UI-heavy, so know these conventions:

- **`render` is a pure function of `App`** (`systui-ui/src/ui.rs::render`). It reads
  state and draws; it must not fetch or mutate. New data must land in `App` first
  (populated from the gather), then be rendered.
- **Tab → renderer dispatch** is a `match` in `render` (~line 370). Each tab has its own
  `render_*` fn; detail panels (`render_service_detail`, `render_container_detail`,
  `render_database_detail`) are the pattern to copy for a Processes detail panel.
- **All colour comes from `theme.rs` tokens** (`app.theme.fg`, `.accent`, `.critical`,
  `.high`/`.medium`/`.low`, `.border`, …). **No inline `Color::Rgb` in screens** — this
  is enforced by the v0.8.2 design and reviewers will reject inline colours.
- **Render tests use `TestBackend`** (`ui.rs` tests, ~line 2956). Add/refresh a render
  test for each tab you change so the layout stays pinned.
- **Real data only.** If a panel has no real collected data, omit it — never mock
  (the v0.8.2/v0.8.4 contract). For the new tasks the data exists (`SystemSnapshot`,
  `process_detail`, `build_process_tree`); wire it, don't fake it.
- **Don't block the UI thread.** Any new fetch (e.g. process detail for the selected
  PID) goes through the v0.8.3 **background refresh** (worker + channel), not a sync call
  inside render or the event loop.

## Current state

`v0.1` → `v1.0` are all complete and tagged on `main`; `v1.0.0` is the latest release.
The per-version detail that used to live here is now in [`CHANGELOG.md`](CHANGELOG.md)
(authoritative) and the per-phase context files in [`docs/phases/`](docs/phases/). Always
cross-check `git log` and `CHANGELOG.md` — this file is a map, not the source of truth.

Highlights worth knowing before touching the code: the action engine and its safety
pipeline (`systui-actions`), the three transports (`Local`/`Ssh`/`Mock`), the truecolor
`theme.rs` + 3-row chrome + multi-panel screens (v0.8.2), the off-thread tiered refresh
with per-collector timeouts (v0.8.3), the local state store for trends/notes/saved
searches (v0.8.4), expected-state policies with `policy.*`/`policy.partial.*` findings
and persisted finding lifecycle states (v0.9).

## Starting a session

The formal version phases (with a `docs/phases/` context file per phase) ran through
v1.0. Post-v1.0 work is feature-driven from [`docs/BACKLOG.md`](docs/BACKLOG.md):

1. Pick a task (e.g. one of the three above) and confirm the plan before writing code.
2. Branch off `main` (e.g. `feat/system-tab-parity`); don't commit straight to `main`.
3. For UI work, read the prototype screenshot for that tab and the *UI implementation
   notes* above before changing `ui.rs`.
4. Implement → run the three quality gates → commit (Conventional Commits, **no
   assistant mention**, no co-author trailers).
5. Update `CHANGELOG.md` (and tick the item in `docs/BACKLOG.md`). For a user-facing
   release, bump the version and tag `vX.Y.Z` (full semver, like `v1.0.0`); the tag
   triggers `.github/workflows/release.yml`.
