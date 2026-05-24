# Phase 8 — v0.8 Fleet

> First session (S8.1) deliverable. Freezes scope and decisions for v0.8 before
> coding. See [`../ROADMAP.md`](../ROADMAP.md) and [`../METHODOLOGY.md`](../METHODOLOGY.md).
> Built on `release/v0.8` (from `main` after v0.7); tag `v0.8` when the DoD is met.

## Goal

Turn SysTUI from a single-server tool into an **infrastructure** tool: see the
state of **10–50 servers without entering each one** (`Product.md` §8 v0.8, §4.16,
§7.7, "Fase 12"). One command opens a **fleet overview** — every inventory host
with its health score, worst findings and a one-line verdict — built by running
the existing per-host review **concurrently** over SSH. From there an operator can
filter by tag, search a service or port across the whole fleet, compare two hosts,
spot basic drift, and export a **single global report**.

```text
systui fleet                 # overview of every inventory host
systui fleet --tag prod      # only hosts tagged `prod`
systui fleet check --security# fleet-wide security-scoped run
```

The substrate already exists: the host **inventory** (`[hosts.<id>]` with `tags`,
`read_only`, `policy`) lives in `systui-core::config`; `resolve_target` maps an id
or `user@host` to a connection; `gather_report` is the headless, transport-agnostic
review of **one** host (system/health, security findings, ports, docker, failed
services, crons, inventory). v0.8 fans that single-host path out across the
inventory and presents the aggregate — it adds **breadth**, not new per-host
collectors.

## In scope

- **Host inventory surface**: read the existing `[hosts.<id>]` inventory as a
  first-class fleet — id, host, tags, `read_only`, `policy`, plus **favorites**
  and **group/tag** selection. A `--tag <t>` (repeatable, AND/OR kept simple)
  filter and an "all hosts" default.
- **Concurrent health checks**: run `gather_report` (or a lighter health-only
  variant) across the selected hosts **concurrently** over `SshTransport`, with a
  bounded concurrency limit, per-host timeouts, and **per-host error isolation** —
  one unreachable or slow host degrades to an error row, never fails the run.
- **Fleet overview**: a sortable table — `id │ tags/role │ health/100 │ findings
  summary (n high / n warn) │ status` — worst-first by default, matching the
  `Product.md` §4.16 mock. A TUI fleet view and a headless table for the CLI.
- **Global search**: find a **service** or an **open port** across all hosts
  ("which hosts run nginx?", "who exposes 6379?") and list the matching hosts.
- **Host comparison**: pick two hosts and show a side-by-side diff of key facts
  (OS/kernel, services, open ports, docker, findings) to answer "why is db-01
  different from db-02?".
- **Basic drift**: compare a host (or the fleet) against a **baseline** —
  initially another host or a captured snapshot — and surface the deltas
  (unexpected/missing ports & services). Lightweight; full policy/expected-state is
  phase 9.
- **Global report**: extend `systui-report` to emit a **fleet report** (one
  document covering the whole selection) in **JSON / Markdown / HTML**, reusing the
  per-host `Report` model and renderers. `systui fleet --export` / a `report`
  multi-host mode.
- **Drill-in**: from the overview, **enter a host** and land in the existing
  single-host TUI (the v0.1–v0.7 experience) over SSH, then come back.

## Out of scope (deferred)

- **Mass destructive / mass mutating actions** across hosts — explicitly forbidden
  this phase (`Product.md` §4.16, "Fase 12": *"No metas ejecución masiva
  destructiva todavía. Solo inspección y reportes."*). Fleet is **inspection &
  reporting only**; the only fleet "action" is *enter a host* and *run a check*.
- **Policy / expected-state engine** (expected/forbidden ports & services,
  thresholds, exceptions, finding states) — phase 9. v0.8 drift is a raw
  host-vs-host / host-vs-snapshot diff, not policy compliance.
- **Persistent fleet history / time-series / trending** and scheduled fleet runs.
- **Command palette** (Ctrl-k) and other cross-cutting UX — separate item.
- **Parallel SSH multiplexing tuning** beyond a simple bounded pool +
  `ControlMaster` reuse note (a v1.0 perf item).
- **Non-SSH transports** for fleet (e.g. agent) — out of product scope.

## Key decisions

- **Fleet = breadth over the existing per-host path.** v0.8 adds no new per-host
  collectors. It calls `gather_report` (the single headless review) per host and
  aggregates. This keeps local↔remote parity and avoids forking collector logic;
  the known `gather_report`/`systui-ui::data` overlap is still tracked, not widened.
- **Concurrency with isolation and bounds.** Hosts are gathered concurrently via
  `tokio` with a **bounded** number of in-flight connections and a per-host
  timeout. Each host result is a `Result` rendered as its own row; a failure is a
  visible "unreachable / partial" row, never a panic or an aborted fleet run
  (graceful degradation, the project-wide rule).
- **Determinism & diff-friendliness.** The overview is **sorted deterministically**
  (worst health first, then id) and the caller injects `generated_at`, so fleet
  reports are reproducible — same as the per-host report contract.
- **Reuse the `Report` model for the global report.** A `FleetReport` is a thin
  aggregate of per-host `Report`s + a summary table; the three existing renderers
  are extended, not replaced. All host-derived text stays **HTML-escaped** (remote
  hosts are untrusted) — the v0.6 security requirement carries over.
- **Reporting/overview stay read-only & safe.** Building the overview and the
  global report is pure inspection; per-host `read_only` profiles are still honored
  when an operator drills into a host.
- **Inventory is the source of truth.** Fleet selection is driven by the existing
  `[hosts.<id>]` config (tags, read_only, policy); favorites are a small additive
  field. No separate fleet datastore this phase.
- **A new `systui-fleet` crate (or `systui-report` fleet module).** Decide in S8.3:
  prefer a dedicated module/crate for the aggregation + overview model so the CLI
  and TUI share one headless fleet path, mirroring how `gather_report` serves both.

## Sessions

- **S8.1 — Context** *(this file)*.
- **S8.2 — Host inventory & selection**: surface the inventory as a fleet —
  tags/groups, favorites, `--tag` filtering; the fleet selection model + tests.
  **Done.** `systui-core::fleet` adds `FleetHost` and `FleetFilter` plus
  `Config::select_fleet`: a pure, I/O-free selection over the `[hosts.<id>]`
  inventory with **OR** tag semantics (repeatable `--tag`), a `favorites_only`
  filter (new `favorite` host field), and a deterministic order (favorites first,
  then id). The `fleet` CLI now lists the resolved selection (`--tag`, repeatable;
  `--favorites`), with empty-inventory and no-match messages. The concurrent health
  gather and overview build on this selection in S8.3.
- **S8.3 — Concurrent health checks + global overview**: bounded concurrent
  `gather_report` across the selection with per-host timeout/error isolation; the
  sortable overview model; `systui fleet` CLI table + TUI fleet view; drill-in to a
  host.
  **Headless overview + CLI done.** `systui-report::fleet` adds a pure, serializable
  `FleetOverview` / `FleetHostSummary` / `FleetOutcome` (decided: a module in
  `systui-report`, reusing `gather_report`, not a new crate) with a **worst-first**
  ordering (failed hosts first, then ascending health, then critical+high, then id)
  and `reviewed`/`failed` constructors + unit tests. The `fleet` CLI gathers the
  selection **concurrently** over `SshTransport` — bounded by a `Semaphore`
  (`FLEET_CONCURRENCY = 8`), each host capped by a `tokio::time::timeout`
  (`FLEET_HOST_TIMEOUT = 30s`) via a `JoinSet` — with full **per-host error
  isolation** (unreachable/auth/timeout → an `ERR` row, never an aborted run),
  honouring per-host `read_only`, and prints the worst-first table. Verified against
  unreachable hosts (isolated `ERR` rows, run completes). **Remaining for S8.3:** the
  interactive **TUI fleet view + drill-in** to a host (the CLI overview already
  satisfies the core "see N servers without entering each" DoD).
- **S8.4 — Global search + host comparison + basic drift**: search a service/port
  across the fleet; side-by-side host comparison; host-vs-host / host-vs-snapshot
  drift deltas.
- **S8.5 — Global reports + polish → tag v0.8**: `FleetReport` over the existing
  JSON/MD/HTML renderers, escaped and self-contained; final polish; merge `--no-ff`
  into `main` + tag `v0.8` (inspection & reporting only — no mass destructive ops).

## Definition of Done

- `systui fleet` (and `--tag <t>`) shows a **fleet overview** of the inventory:
  per-host health score, worst-first findings summary and status, built by
  **concurrent** per-host reviews over SSH with per-host error isolation.
- An operator can **see the state of 10–50 servers without entering each one**
  (`Product.md` "Fase 12" criterion).
- **Global search** finds a service or port across all hosts; **host comparison**
  diffs two hosts; **basic drift** surfaces deltas against a baseline.
- A **global report** (JSON / Markdown / HTML) covers the whole selection, reusing
  the per-host `Report` model and renderers; HTML is self-contained and escaped.
- Drilling into a host opens the existing single-host TUI over SSH, honoring the
  host's `read_only` profile.
- **No mass destructive/mutating fleet actions** exist.
- `cargo fmt --check`, `cargo clippy -D warnings` and `cargo test --workspace` pass;
  any output parsing is fixture/golden-tested.

## Risks & open questions

- **Concurrency blast radius**: gathering 10–50 hosts fires many parallel SSH
  sessions. Mitigate with a bounded pool + per-host timeout; note OpenSSH
  `ControlMaster` reuse as a follow-up. A slow/hung host must not stall the fleet.
- **Partial fleet data**: unreachable hosts, auth failures or limited remote users
  yield gaps. The overview/report must say so per host (status + capability
  metadata) rather than implying a clean fleet.
- **Aggregation cost & report size**: a full per-host `Report` × 50 hosts is large.
  Consider a **lighter health-only gather** for the overview and the full
  `gather_report` only on drill-in/global report; cap sections in the fleet report.
- **Drift scope creep**: "basic drift" can balloon into the phase-9 policy engine.
  Keep v0.8 drift to a raw observed-state diff (host vs host / vs snapshot); defer
  expected/forbidden rules and finding states to phase 9.
- **Collection-path duplication** (carried over from v0.6): keep `gather_report`
  the single headless per-host path; the fleet layer must call it, not re-implement
  collectors.
- **`--tag` semantics**: define AND vs OR for multiple tags up front (start simple:
  repeated `--tag` = OR / "any of") to avoid ambiguous filtering.
