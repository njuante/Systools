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
