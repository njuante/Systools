# Phase 1 — v0.1 Functional demo

> First session (S1.1) deliverable. Freezes scope and decisions for v0.1 before
> coding. See [`../ROADMAP.md`](../ROADMAP.md) and [`../METHODOLOGY.md`](../METHODOLOGY.md).
> Built on `release/v0.1` (continuing from phase 0); tag `v0.1` when the DoD is met.

## Goal

The first visually useful local app: run `systui` on a Linux box and get a fast,
**prioritized** health view — not a metric dump. It must answer "is this server
healthy, and what should I look at first?" using the modules available in v0.1
(`Product.md` §8 v0.1, §15, §4.1, §4.2).

This builds directly on the phase-0 foundation: collectors run through a
`Transport`, the UI renders snapshots and degrades gracefully on missing data.

## In scope

- **System collectors**: OS/distro, kernel, hostname, uptime, CPU usage, RAM, swap,
  load average, per-mount disk usage, logged-in users.
- **Dashboard**: a prioritized overview with a health header, CPU/RAM/swap, disk
  usage and load; plus a "findings/attention" list driven by checks.
- **System detail tab**: the full system metrics.
- **Top processes**: a collector + panel sorted by CPU and by RAM (read-only; no
  signals yet — that's v0.2).
- **Failed systemd units**: a collector + panel listing failed units.
- **Basic journald logs**: recent critical/error log lines in a simple read-only view.
- **Checks**: disk/RAM/swap/load thresholds (from `config.thresholds`) producing
  prioritized warnings.
- **Refresh**: manual (`r`) and configurable auto-refresh (`config.general.default_refresh_seconds`).
- **Minimal Markdown report**: a health snapshot exportable via `systui report`.

## Out of scope (deferred)

- Any **mutating actions** (restart/stop/kill, signals) and the action engine → v0.2.
- Log filters/regex/tailing, error grouping → phase 4.
- Network/exposure, security findings, certs → phase 3.
- Docker, crons, SSH, databases, fleet, policies → later phases.
- Full health-score weighting model can stay simple here; richer scoring later.

## Key decisions

- **Data sources, agentless:** prefer reading `/proc` via `Transport::read_file`
  (`/proc/meminfo`, `/proc/loadavg`, `/proc/uptime`, `/proc/stat`, `/proc/cpuinfo`)
  so the same code works locally and over SSH later. Use commands where `/proc`
  is insufficient: `df -P` (disks), `who` (users), `systemctl --failed` (units),
  `journalctl` (logs). Everything still goes through `CommandSpec`.
- **CPU usage needs sampling:** `/proc/stat` gives cumulative ticks; compute usage
  as a delta between two reads. The collector takes a previous sample (or samples
  twice with a short delay). Decide the exact shape in S1.2; keep it testable from
  two fixed `/proc/stat` snapshots.
- **Snapshot model + controller:** introduce a `SystemSnapshot` (and a dashboard
  aggregate) in `systui-collectors`. Generalise the phase-0 single-collector wiring
  in `systui-ui/data.rs` into a small controller that runs the collectors and
  assembles the snapshot. The UI renders the snapshot; it does not parse anything.
- **Checks are explainable:** each check yields a short, evidence-bearing line
  (e.g. `"/ at 86% (warn ≥ 80%)"`), reused by both the dashboard and the report.
  Health score (if shown) must list what subtracted from it (`Product.md` §7.1).
- **Parsers are tested with fixtures:** every parser (`/proc/*`, `df`, `who`,
  `systemctl --failed`, `journalctl`) gets golden-file/fixture tests under
  `systui-testkit` or the collector crate (`Product.md` §12). Capture real outputs.
- **Graceful degradation:** unreadable sources (permissions, missing `journalctl`)
  render as partial-data/permission-denied, never crash.
- **Auto-refresh:** the event loop already supports `r`; add an elapsed-time tick
  using `default_refresh_seconds`. Pass the loaded `Config` into the UI (today
  `run` ignores it).

## Sessions

- **S1.1 — Context** *(done — this file)*.
- **S1.2 — System collectors** *(done)*: OS/kernel/hostname/uptime, CPU, RAM, swap,
  load, disks, users → `SystemSnapshot`, with fixture-based parser tests.
- **S1.3 — Dashboard & system panels** *(done)*: header, CPU/RAM/swap bars, disk
  usage and load on the dashboard; full system detail tab; the data controller now
  runs `SystemCollector` (the phase-0 `HostInfoCollector` demo was removed).
- **S1.4 — Top processes** *(done)*: `ProcessCollector` (via `ps`) + a Processes
  tab with a sortable table (top 20; `s` toggles CPU/memory ordering).
- **S1.5 — Failed systemd units** *(done)*: `FailedUnitsCollector`
  (`systemctl --failed`) + Services tab listing failed units, plus a failed-unit
  count on the dashboard. Added severity colors (ok/warn/danger) to the theme.
- **S1.6 — Basic journald logs**: recent critical/error lines + simple view.
- **S1.7 — Checks & refresh**: threshold checks feeding the dashboard; manual +
  auto refresh wired to config.
- **S1.8 — Minimal Markdown report** + polish → **tag v0.1**.

## Definition of Done

- `systui` runs locally and shows a prioritized health view (dashboard) plus
  system detail, top processes, failed units and recent critical logs.
- Threshold checks surface disk/RAM/swap/load problems with evidence.
- Manual and auto refresh work; missing data degrades gracefully without crashing.
- `systui report --format markdown` produces a health snapshot.
- All parsers have fixture tests; `cargo fmt --check`, `cargo clippy -D warnings`
  and `cargo test --workspace` pass.

## Risks & open questions

- **CPU sampling delay** can make the first paint feel slow; consider showing the
  snapshot immediately and refining CPU on the next tick.
- **`df`/`who`/`systemctl`/`journalctl` output varies by distro/locale** — pin
  parsing to stable, machine-readable flags where possible (`df -P`,
  `systemctl --failed --no-legend --plain`, `journalctl -o json`/`-p`), and capture
  fixtures per distro.
- **Permissions:** `journalctl` may require group membership; some `/proc` fields
  need root. Define the partial-data UX early.
- **Snapshot vs. per-collector state:** decide whether the controller refreshes all
  collectors together or independently (affects auto-refresh granularity).
