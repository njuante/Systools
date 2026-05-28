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
5. [`docs/BACKLOG.md`](docs/BACKLOG.md) — the post-v1.0 candidates (see *What to build next* below).

## Where the project is now: v1.0.0 shipped

**The v0.1 → v1.0 roadmap is complete.** `v1.0.0` is tagged on `main` and published as a
GitHub Release (static musl binaries, `.deb`/`.rpm`, AUR, `install.sh`, checksums, SBOM,
keyless cosign signature). v1.0 added per-distro integration tests, parser fuzzing, a
large-log benchmark, a security review and packaging — see `CHANGELOG.md` and
`docs/phases/phase-10-v1.0-release.md`. A security/quality audit
([`docs/AUDIT-2026-05.md`](docs/AUDIT-2026-05.md)) found no vulnerabilities.

New work is now **v1.1+** (`docs/ROADMAP.md` "Out of scope until v1.1+").

### In flight: the `feat/tui-polish` branch (post-v1.0, not yet merged)

A UI/UX pass lives on `feat/tui-polish` (see `CHANGELOG.md` "Unreleased"). It is **not yet
merged into `main` or tagged**. It adds:
- a **multi-theme system** (enriched dark / midnight / light) cycled live with `T` and
  persisted to `[general] theme`, plus **per-domain accent colors** (each tab has its hue);
- **richer per-tab detail** wired from data the collectors already gather — Services
  (main PID, unit-file path, dependencies, recent `journalctl -u`), Network (real
  established peers), Docker (published ports, max-retry), System (CPU model, virtualization);
- a **guided, visual cron builder** (frequency presets + live preview of expression /
  description / next runs) replacing the raw cron-expression form.

When this branch merges, fold its highlights into the section above and clear this note.

## What to build next

The full candidate list lives in [`docs/BACKLOG.md`](docs/BACKLOG.md). For UI work, the
approved prototype is in [`docs/interfaz/`](docs/interfaz/) (Ratatui spec + screenshots
under `_extracted/screenshots/`) — **read the prototype screenshot for a tab before
changing it**, and keep **real data only, never mock**.

**Done since v1.0** (don't re-do these): the **System** and **Processes** tabs already
match the multi-panel idiom — System is `render_system` (identity/disks/memory/users) and
Processes has `render_process_detail` plus a tree view (`build_process_tree`). The
`feat/tui-polish` branch above further enriched Services/Network/Docker/System and replaced
the cron form with a guided builder.

### Highest value / lowest effort (reuse existing collectors)

These three reuse collectors that already exist, so they are the cheapest big wins:

1. **TLS/SSL certificate panel** — certs are already discovered and scored as findings
   (`cert_expiry_warning_days`, the v0.3 cert checks). Promote them to a first-class
   sortable view: every cert (local files + probed `host:443`) with days-to-expiry,
   CN/SAN, issuer, coloured by urgency.
2. **Disk drill-down** — System shows only the global disk %. Add an `ncdu`-style
   breakdown of the biggest dirs/files and fastest-growing logs (new `du`/`find`
   `CommandSpec` reads), surfaced as a System sub-panel.
3. **Security updates + reboot-required** — the packages collector already feeds the
   Dashboard UPDATES tile; add how many pending updates are *security* and whether a
   reboot is required (`/var/run/reboot-required`, `needs-restarting -r`).

### Larger / differentiating

- **Auth & access panel** (failed SSH logins, sessions, `authorized_keys`, no-password
  shells) — extends Security and `capabilities`.
- **Live mode + richer sparklines** (per-core CPU, disk I/O, per-interface net, follow-mode
  log tail) — extends the existing trends store.
- **Alerting / thresholds** driving the tab badges and the report.
- **Config/state drift** diffing snapshots across runs (the v0.8.4 store is the substrate).

### Gated "expert console" (free-form shell)

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
