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

- **`v0.8` complete and tagged on `main`.** Each version is built on
  `release/vX.Y` from `main`, then merged `--no-ff` + tagged at the end of its phase.
  `v0.1` through `v0.8` are tagged on `main`.
- **Phase 0 (Foundation) complete**: workspace, contracts, Local/Mock transports,
  CLI + config + tracing, TUI shell, async/sync bridge.
- **Phase 1 / v0.1 complete**: dashboard with health score + findings,
  system/processes/services/logs views, threshold checks, auto-refresh, Markdown report.
- **Phase 2 / v0.2 complete** (S2.1–S2.7): full systemd + process collectors/detail,
  service + signal actions, log filters + incremental search, the **action engine**
  (guardrail → read-only → risk → preview → confirm → execute → verify), the **audit
  log**, and UI action invocation (select + `a` → confirm → run → audited).
- **Phase 3 / v0.3 (Network & security) complete** (S3.1–S3.8): network collectors
  (`ip`/`ss`/resolv.conf → `NetworkSnapshot`), port→process→systemd-unit correlation
  (via `/proc/<pid>/cgroup`), the **exposure map** (risk-ranked listeners with evidence),
  connectivity tools (ping/DNS/TCP connect), the shared `core::Finding` model +
  `systui-security` posture checks (SSH, sudo, failed logins, firewall, file perms,
  docker socket, SUID, exposed ports) via `security_scan`, certificate checks (local +
  remote `host:443` over `openssl`), and the **Network/Security TUI tabs** + dashboard
  security summary. Core enabler added: `CommandSpec::stdin` (pipe without a shell).
- **Phase 4 / v0.4 (Docker & crons) complete** (S4.1–S4.7): Docker collectors
  (`DockerCollector`, `container_stats`, `inspect_container` → `InspectSummary`),
  Docker ops (`DockerAction` start/stop/restart/remove + `container_logs`), Docker
  risk checks (`docker_findings`: privileged, docker.sock/dangerous mounts, sensitive
  published ports via reused `exposure_map`, unhealthy, restart loops, `latest`, no
  mem limit), cron sources (`cron.rs`: `parse_crontab` + `collect_cron_entries`),
  timers & a self-built cron evaluator (`parse_schedule`/`CronSchedule` with
  `describe`/`next_after`/`upcoming`, `collect_timers`/`SystemdTimer`), **cron risk
  checks** (`systui-security::cron`: missing/non-exec script, world-writable or
  `/tmp` root script, no logging, invalid schedule, high frequency, duplicates), and
  the **Docker + Crons TUI tabs** (container list + detail/risks; cron jobs with NL
  schedule/next-run + timers + warnings) with **dashboard Docker/Crons hooks**.
  Docker + cron findings are merged into the shared findings list; `a` on the Docker
  tab plans a state-aware lifecycle action through the engine.
- **Phase 5 / v0.5 (Remote SSH) complete** (S5.1–S5.5): **`SshTransport`** over the
  system OpenSSH client implementing the full `Transport` contract (key/agent auth,
  custom port, timeouts, stdin forwarding) with a single fixture-tested POSIX
  shell-quoting layer at the SSH boundary and error mapping mirroring
  `LocalTransport`; `Config::resolve_target` (inventory id or `user@host`) wiring
  `systui ssh <target>` to the same TUI as local, honouring per-host `read_only`;
  **`HostCapabilities`/`probe_capabilities`** (root/sudo detection) that degrade
  `Privileged → SafeActions` for a non-privileged user and label the title bar.
  Parity audit confirmed every module runs over SSH unchanged (all host access is
  via the `Transport`); the UI is identical local vs remote apart from the host
  label. Note: v0.5 requires non-interactive auth (`BatchMode`); a native Rust SSH
  backend remains deferred but enabled by the trait boundary.
- **Phase 6 / v0.6 (Reports) complete** (S6.1–S6.5): a serializable **`Report`**
  model + **`gather_report`** (the headless equivalent of the dashboard refresh over
  any `Transport`) in `systui-report`, with three pure renderers — **`to_json`**
  (full structured model), **`to_markdown`** and **`to_html`** (a single
  self-contained, escaped, printable file) — covering executive summary, health,
  security findings (evidence + recommendations), open ports, docker, failed
  services, crons, host inventory, recommendations and notes. A **`ReportScope`**
  powers `--security`. The **`report` CLI** (`systui report [--host <id|user@host>]
  --format markdown|json|html [--security] [-o FILE] [--note TEXT]…`) runs locally or
  over the v0.5 SSH transport, honouring per-host `read_only`. All host-derived text
  is HTML-escaped.
- **Phase 7 / v0.7 (Databases) complete** (S7.1–S7.4): database discovery and
  operational visibility for PostgreSQL, Redis, MySQL/MariaDB and MongoDB over the
  shared `Transport`: service/unit state, listeners, process ownership, version,
  exposure classification, safe credential-source labels (local sockets, `.pgpass`,
  `mysql_config_editor`, redacted env presence) and best-effort operational signals
  (connections, sizes, replication, locks/blocked clients and recent errors).
  Database findings (`db.*`) cover public exposure, Redis auth risk when no
  credential source is detected, blocked work, broken replication and recent
  errors. The TUI includes a **Databases** tab; JSON/Markdown/HTML reports include a
  Databases section. No database password is stored or rendered.
- **Phase 8 / v0.8 (Fleet) complete** (S8.1–S8.5): the inventory becomes a fleet.
  `systui-core::fleet` selects hosts by tag (OR) / favorites with a deterministic
  order. `systui fleet` gathers the selection **concurrently** over SSH (a bounded
  `Semaphore` + per-host `timeout`, full per-host error isolation) into a
  `FleetReview` that keeps each host's full `Report`; the worst-first `FleetOverview`
  is derived from it. On a terminal it opens a read-only **fleet TUI**
  (`systui-ui::fleet`, selectable table, `r` refresh, `Enter` to **drill into** a
  host's per-host TUI over SSH), and prints a table when piped. From the same single
  gather: **global search** (`--search`, by port or service substring), **host
  comparison** (`--compare A B`, side-by-side + ports/services **drift** deltas via
  `HostFacts`/`HostComparison`), and a **fleet report** (`--format json|markdown|html
  [-o FILE]`) — JSON is the full model, MD/HTML a digest, escaped and self-contained.
  Inspection & reporting only — no mass destructive actions. (Host-vs-snapshot drift
  deferred to phase 9.)
- **Phase 8.5 / v0.8.1 (In-TUI management & UX polish) — IN PROGRESS** on
  `release/v0.8.1` (an intermediate phase before Policies; see
  `docs/phases/phase-08-1-v0.8.1-management-ux.md`). Done so far:
  - **S8b.2** — config persistence: `systui-storage::save_host[_to]` /
    `remove_host[_from]` edit only the `[hosts.<id>]` table via `toml_edit`
    (preserving comments/other tables, atomic temp+rename); `Config::upsert_host` /
    `remove_host` in-memory.
  - **S8b.3** — host management in the fleet TUI: reusable `systui-ui::form` modal;
    `a` add / `e` edit (id fixed, policy preserved) / `d` delete inventory hosts,
    validated, persisted, mirrored in-memory, re-gathered without leaving the screen;
    disabled in read-only mode. `run_fleet` now takes `&mut Config` + config path.
  - **S8b.4** — `systui-actions::cron::CronAction` (add/edit/delete/enable/disable on
    the **user crontab**) through the engine: schedule validated in `preview`, prior
    crontab backed up to `/tmp/systui-crontab.bak` via `tee`, installed via
    `crontab -` piping content through `CommandSpec::stdin`. Pure transforms tested.
  - **TODO — S8b.5**: wire the form + `CronAction` into the **Crons tab** TUI (`a`
    add, `e` edit, `d` delete, toggle enable/disable) through the engine
    (preview/confirm/audit), refresh after. The cron *actions* exist; the **TUI
    wiring in the Crons tab is not done yet**.
  - **TODO — S8b.6**: TUI **layout polish** (header/status bar, borders/spacing,
    severity badges, readable tables, loading/empty/error states) → then merge
    `--no-ff` into `main` + tag `v0.8.1`.
- **Next: Phase 9 / v0.9 — Policies & expected state** (after v0.8.1 is tagged).
  Start with the phase context file (`docs/phases/`) before any code, per the
  methodology. Read `docs/ROADMAP.md` for the v0.9 scope.

## Starting a session

Confirm the session plan before writing code, then: implement → run the three
quality gates → commit (Conventional Commits, no assistant mention) → update the
phase context file's session status.
