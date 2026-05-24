# Phase 4 — v0.4 Docker & crons

> First session (S4.1) deliverable. Freezes scope and decisions for v0.4 before
> coding. See [`../ROADMAP.md`](../ROADMAP.md) and [`../METHODOLOGY.md`](../METHODOLOGY.md).
> Built on `release/v0.4` (from `main` after v0.3); tag `v0.4` when the DoD is met.

## Goal

Cover the two things that actually run modern servers and constantly break:
**containers** and **scheduled jobs**. SysTUI should let an operator review a
Docker host and a crontab and immediately see *what is running, what is risky and
what is broken* — privileged containers, `docker.sock` mounts, sensitive published
ports, unhealthy/restart-looping containers, crons pointing at missing scripts, and
jobs with no logging — without juggling `docker` and `crontab` commands
(`Product.md` §4.7, §4.8, §8 v0.4).

This phase keeps the established split: **collectors read**, the **action engine**
runs the few lifecycle mutations (Docker start/stop/restart/remove), and risk
checks produce shared `Finding`s. Crons are **read-only/detection** this phase.

## In scope

- **Docker collectors**: list containers (running + stopped) with image, state,
  status, health, published ports, created; per-container **inspect summary**
  (privileged, mounts, restart policy, memory limit, networks) and **stats**
  (CPU/RAM, no-stream snapshot).
- **Docker operations**: view recent **logs**, list ports/volumes/networks, and
  **start / stop / restart / remove** through the existing action engine (remove
  requires typed confirmation; all run via `CommandSpec`).
- **Docker risk checks** (→ `Finding`s): `privileged`, `docker.sock` mounted,
  dangerous bind mounts (`/`, `/etc`, `/var/run`), sensitive published ports,
  `unhealthy`, restart loops (high `RestartCount`), `latest` image tag, no memory
  limit.
- **Cron sources**: per-user `crontab -l`, `/etc/crontab`, `/etc/cron.d/*`, and
  `/etc/cron.{hourly,daily,weekly,monthly}`; **systemd timers** via
  `systemctl list-timers`.
- **Cron parsing & validation**: parse the 5-field expression + command, validate
  it, render a natural-language preview, and compute the next run(s).
- **Cron checks** (→ `Finding`s): script does not exist, script not executable,
  no stdout/stderr redirection (no logging), duplicate entry, root cron writing to
  insecure paths, suspiciously high frequency.
- **TUI**: a **Docker tab** (container list + detail/risks) and a **Crons tab**
  (jobs/timers with next-run and warnings); dashboard hooks for problematic
  containers/crons. Docker findings feed the existing Security panel.

## Out of scope (deferred)

- **Cron mutation** (create / edit / delete / temporarily disable) — detection only
  now; editing is risky and lands with policies/actions in a later phase.
- **Kubernetes** (k3s/minikube) and **Podman** — explicitly not in the first
  Docker version (`Product.md` §4.7); only the `docker` CLI.
- **Compose**: only a summary (project/label), not a full compose project view.
- **Shell into container** and **real-time/streaming logs** — we show a bounded
  recent-logs snapshot; an interactive shell is future work.
- **Excessive-log-size** check — deferred unless cheap to read (needs log path/size
  introspection); keep it out of the initial check set.
- Remote/SSH — phase 5. Everything must still run over `Transport` so it works
  remotely later for free.

## Key decisions

- **Docker data source:** the `docker` CLI through `Transport`/`CommandSpec`, using
  machine-readable output: `docker ps -a --no-trunc --format '{{json .}}'` (one JSON
  object per line), `docker inspect <id>` (JSON array) and `docker stats --no-stream
  --format '{{json .}}'`. Logs via `docker logs --tail N --timestamps <id>`. No
  shell pipes — each is a single `CommandSpec`.
- **Docker availability / degradation:** probe once (e.g. `docker version`); if the
  binary is missing or the socket is not accessible (rootless/permission), the
  Docker views degrade to a clear "Docker unavailable / partial" state, never a
  crash — same contract as every other module.
- **Docker actions:** reuse the action engine and its guardrails (read-only mode
  blocks them; risk → preview → confirm → execute → verify → audit). `remove` is
  destructive → typed confirmation + treated as high risk. Logs and inspect are
  read-only and need no confirmation.
- **Docker risk checks live in `systui-security`** as pure functions over the
  inspect/ps/stats data, producing `core::Finding`s (reusing `Severity` and the
  sensitive-port set from v0.3). Container findings join the Security panel and
  dashboard summary built in S3.8.
- **Cron parsing is ours** (pure, fixture-tested): tokenize the 5 schedule fields +
  command, support `@hourly`/`@daily`/… macros, validate ranges/steps, and produce a
  human-readable description. **Next-run computation** uses a vetted cron crate
  (candidate: `cron`/`croner`, evaluated against `chrono` in S4.6); chosen crate is
  recorded there before use.
- **Cron checks** are pure functions over parsed entries plus a few transport
  lookups (does the script path exist / is it executable, via `Transport::file_exists`
  and a `stat`/perms read reused from v0.3's `parse_stat`).
- **Module placement:** Docker + cron *collectors* in `systui-collectors`
  (`docker.rs`, `cron.rs`); Docker lifecycle *actions* in `systui-actions`; Docker
  and cron *risk checks* in `systui-security`; tabs in `systui-ui`. New `ModuleId`s
  already exist (`Docker`, `Crons`). No new crate.
- **Parsers tested with fixtures:** captured `docker ps/inspect/stats` JSON, `docker
  logs`, crontab files, `systemctl list-timers` output (`Product.md` §12).

## Sessions

- **S4.1 — Context** *(this file)*.
- **S4.2 — Docker collectors**: containers (running/stopped), stats, inspect summary.
- **S4.3 — Docker ops**: logs, ports, volumes, networks; start/stop/restart/remove.
- **S4.4 — Docker risk checks**: privileged, docker.sock mount, dangerous mounts,
  sensitive published ports, unhealthy, restart loops, `latest` tag, no mem limit.
- **S4.5 — Cron sources**: user crontab, `/etc/crontab`, `/etc/cron.d`, `cron.*`.
- **S4.6 — Timers & validation**: systemd timers, cron expression validation,
  next-run preview.
- **S4.7 — Cron checks** (missing script, no exec perms, no logging, duplicate,
  risky root) + Docker/Crons tabs polish → **tag v0.4**.

## Definition of Done

- The Docker tab lists running and stopped containers with image, state, health and
  published ports; a detail view shows the inspect summary, stats and risks.
- Docker start/stop/restart/remove work through the action engine (remove confirmed),
  are audited, and are blocked in read-only mode.
- Docker risk checks surface privileged containers, `docker.sock`/dangerous mounts,
  sensitive published ports, unhealthy/restart-looping containers, `latest` tags and
  missing memory limits as evidence-based findings.
- The Crons tab lists cron jobs (per-user + system) and systemd timers with a
  natural-language schedule, next run, and warnings.
- Cron checks detect missing/non-executable scripts, missing logging, duplicates and
  risky root jobs.
- All new parsers have fixture tests; `cargo fmt --check`, `cargo clippy -D warnings`
  and `cargo test --workspace` pass. Everything runs through `Transport`.

## Risks & open questions

- **`docker` output varies** by version and may need root/socket access; prefer
  `{{json .}}` formats and capture fixtures. `docker stats` can be slow → use
  `--no-stream` and a timeout.
- **Rootless / permission**: distinguish "docker not installed" from "no permission
  to the socket" so the UI message is honest (don't claim "no containers").
- **Cron next-run** depends on the chosen crate's timezone handling; cron runs in the
  host's local time while we compute in `chrono` — pick and document the tz approach
  in S4.6 to avoid off-by-hours previews.
- **User crontabs**: enumerating every user's crontab may need root (`/var/spool/cron`
  is not world-readable); degrade to the current user + system locations otherwise.
- **Duplicate/"high frequency" heuristics** risk false positives; keep them Low/Info
  and evidence-based, and make thresholds explicit.
