# SysTUI — Roadmap to v1.0

This document maps the product design in [`Product.md`](../Product.md) into a concrete,
trackable development plan: **phases → versions → sessions**.

- The product vision, architecture and module specs live in `Product.md`.
- The working method (branching, sessions, commits, Definition of Done) lives in
  [`METHODOLOGY.md`](METHODOLOGY.md).
- The detailed context for each phase lives in [`docs/phases/`](phases/).

---

## Phase ↔ Version model

Each **development phase** is delivered as a **version** and is built on its own
`release/vX.Y` branch. **Phase 0 (Foundation)** has no public release; it is the
substrate for `v0.1` and is built on the `release/v0.1` branch as the first sessions.

| Phase | Version | Theme | Branch |
|-------|---------|-------------------------------|------------------|
| 0     | —       | Foundation & architecture     | `release/v0.1`   |
| 1     | v0.1    | Functional demo               | `release/v0.1`   |
| 2     | v0.2    | Local operation               | `release/v0.2`   |
| 3     | v0.3    | Network & security            | `release/v0.3`   |
| 4     | v0.4    | Docker & crons                | `release/v0.4`   |
| 5     | v0.5    | Remote SSH                    | `release/v0.5`   |
| 6     | v0.6    | Reports                       | `release/v0.6`   |
| 7     | v0.7    | Databases                     | `release/v0.7`   |
| 8     | v0.8    | Fleet                         | `release/v0.8`   |
| 8.5   | v0.8.1  | In-TUI management & UX polish | `release/v0.8.1` |
| 8.6   | v0.8.2  | UI redesign (approved design) | `release/v0.8.2` |
| 8.7   | v0.8.3  | Optimization (performance)    | `release/v0.8.3` |
| 9     | v0.9    | Policies & expected state     | `release/v0.9`   |
| 10    | v1.0    | Stabilization & release       | `release/v1.0`   |

> The first session of every phase creates that phase's context file in
> `docs/phases/` before any code is written.

---

## Phase 0 — Foundation & architecture (→ substrate for v0.1)

**Goal:** a robust, well-separated base. Stable empty TUI, mocked collectors,
config loading, no crashes on error. Maps to `Product.md` §6 "Fase 0–1" and §14 steps 1–6.

Sessions:
- **S0.1 — Context** *(this is the first session)*: write `phase-00-foundation.md`.
- **S0.2 — Workspace scaffold**: Cargo workspace + all crates (`systui-cli`, `-core`,
  `-ui`, `-transport`, `-collectors`, `-actions`, `-security`, `-report`, `-storage`,
  `-testkit`), CI skeleton, `rustfmt`/`clippy` config.
- **S0.3 — Core contracts**: `systui-core` — typed errors (`thiserror`), `CommandSpec`,
  `CommandOutput`, domain models, config schema, `Collector`/`Action` traits.
- **S0.4 — Transport layer**: `Transport` trait + `LocalTransport` + `MockTransport`.
- **S0.5 — CLI & config**: `clap` CLI (subcommands skeleton), config load/merge,
  `tracing` logging, exec modes (read-only / safe / privileged) plumbing.
- **S0.6 — TUI shell**: `ratatui`/`crossterm` event loop, app state, navigation
  (tabs), theme, key bindings, empty dashboard, UI states (loading/error/empty).
- **S0.7 — Wire-up & tests**: render a mocked collector end-to-end; unit tests in
  `systui-testkit`; quality gates green.

**Definition of Done:** `systui` opens a stable TUI, loads config, runs mocked
collectors, shows errors without crashing, has basic unit tests passing.

---

## Phase 1 — v0.1 Functional demo

**Goal:** first visually useful local app. (`Product.md` §8 v0.1, §15)

Sessions:
- **S1.1 — Context**: `phase-01-v0.1-demo.md`.
- **S1.2 — System collectors**: OS, kernel, hostname, uptime, CPU, RAM, swap, load,
  disks, logged users.
- **S1.3 — Dashboard panels**: health header, CPU/RAM/swap, disk usage, load.
- **S1.4 — Top processes**: collector + panel (sort by CPU/RAM).
- **S1.5 — Failed systemd units**: collector + panel.
- **S1.6 — Basic journald logs**: read recent critical logs, simple view.
- **S1.7 — Checks & refresh**: disk/RAM/swap/load thresholds, manual + auto refresh.
- **S1.8 — Minimal Markdown report** + polish → **tag v0.1**.

**DoD:** `systui` runs locally and gives a fast, prioritized health view.

---

## Phase 2 — v0.2 Local operation

**Goal:** SysTUI becomes usable for real local ops. (`Product.md` §8 v0.2, §10 action engine)

Sessions:
- **S2.1 — Context**: `phase-02-v0.2-local-ops.md`.
- **S2.2 — systemd module**: list/filter (active/failed/enabled/disabled), detail,
  unit file, dependencies, start/stop/restart/reload/enable/disable/mask/unmask.
- **S2.3 — Processes module**: full list, tree, signals (SIGTERM/SIGKILL/SIGHUP),
  detail (cmd/cwd/open files/ports), critical-process guardrails.
- **S2.4 — Logs filters**: level, date, service, regex, incremental search.
- **S2.5 — Action engine**: permission → read-only → risk → preview → confirm →
  backup → execute → verify → audit (per `Product.md` §10).
- **S2.6 — Read-only mode + audit log**: JSON audit trail, read-only enforcement.
- **S2.7 — Contextual confirmations** + safe-action guardrails + polish → **tag v0.2**.

**DoD:** find a failed service, read its logs, restart it safely; all mutations audited.

---

## Phase 3 — v0.3 Network & security

**Goal:** first differentiating value. (`Product.md` §8 v0.3, §4.6, §4.10, §4.12, §6 Fase 6)

Sessions:
- **S3.1 — Context**: `phase-03-v0.3-net-security.md`.
- **S3.2 — Network collectors**: interfaces, IPs, routes, DNS, ports, connections.
- **S3.3 — Correlation**: port → process → systemd service.
- **S3.4 — Exposure map**: 0.0.0.0 listeners, sensitive-port checks, risk levels.
- **S3.5 — Connectivity tools**: ping, DNS lookup, TCP connect test.
- **S3.6 — Findings & security checks**: `Finding` model + severity; SSH root/password
  auth, sudo users, failed logins, firewall detection, SUID, critical file perms,
  docker socket.
- **S3.7 — Certificates**: local cert discovery + remote `host:443`, expiry/CN/issuer checks.
- **S3.8 — Security panel** + polish → **tag v0.3**.

**DoD:** SysTUI can show what is exposed, which process exposes it, and a prioritized
risk list with evidence.

---

## Phase 4 — v0.4 Docker & crons

**Goal:** cover modern real-world servers. (`Product.md` §8 v0.4, §4.7, §4.8)

Sessions:
- **S4.1 — Context**: `phase-04-v0.4-docker-crons.md`.
- **S4.2 — Docker collectors**: containers (running/stopped), stats, inspect summary.
- **S4.3 — Docker ops**: logs, ports, volumes, networks; start/stop/restart/remove.
- **S4.4 — Docker risk checks**: privileged, docker.sock mount, dangerous mounts,
  sensitive published ports, unhealthy, restart loops, `latest` tag, no mem limits.
- **S4.5 — Cron sources**: user crontab, `/etc/crontab`, `/etc/cron.d`, `cron.*`.
- **S4.6 — Timers & validation**: systemd timers, cron expression validation, next-run preview.
- **S4.7 — Cron checks** (missing script, no exec perms, no logging, duplicate, risky root)
  + polish → **tag v0.4**.

**DoD:** review a Docker host and find problematic containers/crons without juggling commands.

---

## Phase 5 — v0.5 Remote SSH

**Goal:** become a real remote administration tool. (`Product.md` §8 v0.5, §6 Fase 9)

Sessions:
- **S5.1 — Context**: `phase-05-v0.5-ssh.md`.
- **S5.2 — SshTransport**: key auth, SSH agent, custom port, timeouts.
- **S5.3 — Host profiles**: profiles, known hosts, reconnect.
- **S5.4 — Remote read-only + permission detection**.
- **S5.5 — Local/remote parity**: verify all existing modules work over SSH + polish → **tag v0.5**.

**DoD:** everything that works locally works remotely when the user has permissions;
UI is identical local vs remote.

---

## Phase 6 — v0.6 Reports

**Goal:** useful for documentation, audits, handover. (`Product.md` §8 v0.6, §4 §11)

Sessions:
- **S6.1 — Context**: `phase-06-v0.6-reports.md`.
- **S6.2 — Report model + JSON** export.
- **S6.3 — Markdown reports**: health, security, open ports, docker, failed services, inventory.
- **S6.4 — HTML reports**.
- **S6.5 — `report` CLI subcommand** + session notes + polish → **tag v0.6**.

**DoD:** deliver a useful report after reviewing a server (`systui report --host ... --format ...`).

---

## Phase 7 — v0.7 Databases

**Goal:** operational visibility for critical services (not a SQL client). (`Product.md` §8 v0.7, §4.9)

Sessions:
- **S7.1 — Context**: `phase-07-v0.7-databases.md`.
- **S7.2 — Detection & exposure**: PostgreSQL, Redis, MySQL/MariaDB, MongoDB —
  service/port/version/state/exposure.
- **S7.3 — Operational panels**: connections, sizes, recent errors, replication.
- **S7.4 — DB checks + credentials handling** (env/socket/.pgpass/prompt, never plaintext)
  + polish → **tag v0.7**.

**DoD:** detect operational DB problems in seconds without replacing `psql`/`mysql` CLI.

---

## Phase 8 — v0.8 Fleet

**Goal:** from single-server to infrastructure tool. (`Product.md` §8 v0.8, §4.16)

Sessions:
- **S8.1 — Context**: `phase-08-v0.8-fleet.md`.
- **S8.2 — Host inventory**: tags, groups, favorites.
- **S8.3 — Concurrent health checks** + global overview.
- **S8.4 — Global search + host comparison + basic drift**.
- **S8.5 — Global reports** + polish → **tag v0.8** (inspection & reporting only, no mass destructive ops).

**DoD:** see the state of 10–50 servers without entering each one.

---

## Phase 8.5 — v0.8.1 In-TUI management & UX polish

**Goal:** make the inventory and the user crontab **manageable from inside the TUI**
(not just viewable), and give the interface a layout polish. An intermediate phase
inserted between Fleet (v0.8) and Policies (v0.9). Detailed scope in
[`phases/phase-08-1-v0.8.1-management-ux.md`](phases/phase-08-1-v0.8.1-management-ux.md).

Sessions:
- **S8b.1 — Context**: `phase-08-1-v0.8.1-management-ux.md` + this insert.
- **S8b.2 — Config persistence**: write `[hosts.<id>]` back to `config.toml` via
  `toml_edit` (surgical, comment-preserving) + `systui-core` upsert/remove helpers.
- **S8b.3 — Host management in the fleet TUI**: reusable form modal; add/edit/delete
  inventory hosts; persist + re-gather; read-only guard.
- **S8b.4 — Cron actions**: `CronAction` add/edit/delete/toggle on the **user
  crontab** via the action engine (validate → preview → confirm → backup → audit),
  built on `CommandSpec::stdin`.
- **S8b.5 — Cron management in the Crons tab**: wire the form + `CronAction` into the
  Crons tab.
- **S8b.6 — Layout polish + close**: header/status bar, borders/spacing, severity
  badges, readable tables, loading/empty/error states → **tag v0.8.1**.

**DoD:** add/edit/delete SSH hosts (saved to `config.toml`) and add/edit/delete/
toggle user-crontab entries (engine-mediated, validated, backed up, audited) from the
TUI; system cron + timers stay read-only; mutations respect read-only mode; the TUI
layout is visibly cleaner with render tests still passing.

> System-wide cron (`/etc/cron.d`, timers), config sections other than `[hosts.*]`,
> command palette and theme switching are **out of scope** (deferred).

---

## Phase 8.6 — v0.8.2 UI redesign

**Goal:** adopt the **approved visual design** in [`interfaz/`](interfaz/) (a Ratatui
spec + reference screenshots): a truecolor single-source theme, a 3-row chrome
(host-attached bar + health gauge + mode badge), numbered tabs with count badges, and
multi-panel screens matching the prototype. A **visual/structural** phase — it reskins
and re-lays-out existing screens and reuses all current data, collectors and the
action engine unchanged. Detailed scope in
[`phases/phase-08-2-v0.8.2-ui-redesign.md`](phases/phase-08-2-v0.8.2-ui-redesign.md).

Sessions:
- **S8c.1 — Context + foundation**: `phase-08-2-v0.8.2-ui-redesign.md` + this insert;
  truecolor theme tokens; the 3-row chrome (top bar / tabs-with-badges / status bar);
  **Dashboard** rebuilt to its multi-panel recipe (metric tiles + health score +
  findings + at-a-glance) with CPU/RAM history sparklines.
- **S8c.2 — Services & Docker**: tables with severity dots + detail/risk side panels.
- **S8c.3 — Logs**: live-tail with level badges, error-fingerprint panel, follow dot.
- **S8c.4 — Network & Crons**: exposure table + interfaces/firewall panels; cron table
  with severity left-bar + schedule preview + backup callout.
- **S8c.5 — Security & Hosts**: security score header + evidence-block findings; the
  Hosts/fleet card grid.
- **S8c.6 — Polish + close**: spacing/alignment, help overlay, render-test refresh →
  **tag v0.8.2**.

**DoD:** the TUI matches the prototype's visual identity (truecolor theme, 3-row
chrome, badged tabs, multi-panel screens); all colour comes from `theme.rs` tokens
(no inline `Color::Rgb` in screens); every screen renders **real data** (nothing
mocked); behaviour, keymaps and the safety model are unchanged; render stays a pure
function of `App` with `TestBackend` tests passing.

> Command palette / fuzzy search, mouse hit-testing, theme switching and any new
> collectors are **out of scope** (deferred).

---

## Phase 8.7 — v0.8.3 Optimization

**Goal:** make SysTUI **fast and fluid**, locally and especially over SSH — a **pure
performance** phase, no new features or visual changes. The v0.8.2 redesign exposed
real performance problems in remote use: the refresh is synchronous on the UI thread
(so the TUI freezes for the gather's duration), collectors run sequentially, and
every tick re-collects everything. SSH connection multiplexing (landed in v0.8.2)
removed the per-command handshake; this phase removes the remaining stalls above it.
Detailed scope in
[`phases/phase-08-3-v0.8.3-optimization.md`](phases/phase-08-3-v0.8.3-optimization.md).

Sessions:
- **S8d.1 — Context + measurement**: `phase-08-3-v0.8.3-optimization.md` + this insert;
  a timing harness (refresh + per-collector cost) and captured local/SSH baselines.
- **S8d.2 — Background refresh**: move the gather off the UI thread (worker + channel);
  the loop stays responsive with a refresh indicator; manual + auto refresh through it.
- **S8d.3 — Concurrent collectors**: gather independent collectors concurrently,
  preserving real dependencies and deterministic output.
- **S8d.4 — Tiered refresh + caching**: split slow-changing vs live data; collect the
  slow set rarely; less work per tick.
- **S8d.5 — Command batching + cancellation/timeouts**: cut round-trips; cancellable
  in-flight refresh with per-collector timeouts → partial data, never a stall.
- **S8d.6 — Build profile + close**: release-profile tuning, final before/after
  numbers → **tag v0.8.3**.

**DoD:** the TUI never freezes on refresh (local or SSH) — input/redraw stay live with
a refresh indicator; independent collectors run concurrently; slow-changing data is
not re-collected every tick; a slow/unreachable host degrades to partial data on
timeout without stalling; before/after measurements recorded; behaviour, screens,
keymaps and the safety model unchanged; render stays a pure function of `App`; gates
green.

> No UI/visual change beyond a refresh indicator, no new collectors/actions/data, and
> no SSH-library rewrite — those are **out of scope**.

---

## Phase 9 — v0.9 Policies & expected state

**Goal:** validate configuration, not just observe. (`Product.md` §8 v0.9, §4.15, §7.5)

Sessions:
- **S9.1 — Context**: `phase-09-v0.9-policies.md`.
- **S9.2 — Policy schema**: expected/forbidden ports & services, thresholds, sudo users,
  expected certs/containers.
- **S9.3 — Policy evaluation engine** + drift findings.
- **S9.4 — Exceptions + finding states** (open/accepted/ignored/fixed/false-positive)
  + polish → **tag v0.9**.

**DoD:** SysTUI validates servers against expected state and reports drift.

---

## Phase 10 — v1.0 Stabilization & release

**Goal:** real product quality. (`Product.md` §8 v1.0, §6 Fase 14–15, §12)

Sessions:
- **S10.1 — Context**: `phase-10-v1.0-release.md`.
- **S10.2 — Test coverage**: unit tests, fixtures per distro, golden files for parsers.
- **S10.3 — Integration tests**: containers (Debian/Ubuntu/Arch/Fedora/Alpine), systemd VM.
- **S10.4 — Hardening**: parser fuzzing, large-log benchmarks, security review.
- **S10.5 — Packaging**: static x86_64/aarch64 binaries, AUR, `.deb`, `.rpm`,
  `cargo install`, `install.sh`, checksums/signatures/SBOM.
- **S10.6 — Docs & launch**: man page, examples, README, demo GIF, changelog, CI/CD
  → **tag v1.0**.

**DoD (per `Product.md` §16):** stable local + remote modes, smart dashboard, system/
processes/services/logs/network/docker/crons/security/certs/packages, reports
(JSON/MD/HTML), profiles, read-only, audit log, action safety engine, basic policies,
Linux packaging, full docs, tests per main distro.

---

## Out of scope until v1.1+

Advanced fleet, complex auto-remediation, Kubernetes, public plugin SDK, optional
agent, continuous alerting, secret-manager integrations, advanced PDF, web dashboard.
