# Phase 8.8 — v0.8.4 Prototype data parity & persistence

> First session deliverable. Freezes scope and decisions for v0.8.4 before coding.
> An **intermediate** phase between v0.8.3 (optimization) and v0.9 (Policies).
> See [`../ROADMAP.md`](../ROADMAP.md) and [`../METHODOLOGY.md`](../METHODOLOGY.md).
> Built on `release/v0.8.4` (from `main` after v0.8.3); tag `v0.8.4` when the DoD is met.

## Goal

Fill in the **panels the v0.8.2 reskin intentionally left out**. The approved prototype
in [`../interfaz/`](../interfaz/) shows several sections that v0.8.2 omitted because
SysTUI either did not collect that data or did not persist it — the reskin's contract
was *match the layout, but only with real data, never mock*. This phase supplies the
**real data and persistence** behind those omitted panels so the TUI finally reaches
visual+data parity with the prototype.

Unlike v0.8.2 (a pure reskin) this is a **feature/data phase**: it wires up already-
collected-but-unshown data, adds a few honest new collectors, adds one local store for
the persistence-backed panels, and adds two small mutations through the existing action
engine. It does **not** change the visual identity, the chrome, the theme tokens, or the
safety model.

The omitted panels, audited against the current code, fall into four buckets:

1. **Already collected, just not wired into the redesigned screen.**
   - **Services** — the prototype's `ALL / RUNNING / INACTIVE / ENABLED / FAILED`
     filters. `service.rs` already has a full `list-units --type=service --all`
     collector *and* a failed-only one; the redesign showed failed-only.
   - **Network → Connectivity tests** — ping / DNS / TCP probes. `connectivity.rs`
     already implements these as on-demand diagnostics; they are simply not surfaced
     in the redesigned Network screen.
2. **Honest new collectors.**
   - **Network → Firewall** — backend (nftables / iptables / ufw / firewalld), table /
     chain / rule counts, and the "rule for port N but no process bound" /
     "rule conflicts with a listening port" notes.
   - **Docker → Compose projects** — discovered compose projects and their services.
   - **Docker → Image hygiene** — image count, total size, dangling count, last prune.
   - **Crons → Anacron** — `/etc/anacrontab` entries alongside cron + timers.
   - **Dashboard → UPDATES tile** — pending package updates + security updates
     (apt / dnf / pacman / zypper).
3. **Persistence-backed panels (new local store).**
   - **Dashboard → health trend** ("was 91 7d ago") and **Security → score trend**
     ("↓4 from last week") — require storing health-score / finding-count snapshots
     over time.
   - **Dashboard → Session notes** ("+ ADD NOTE") — per-host free-text notes.
   - **Logs → Saved searches** — persisted log filter expressions.
4. **Two small mutations** through the action engine.
   - **Docker → prune dangling images** (`prune dangling…` button).
   - **Crons → run now** (the `run now` action in the cron Actions panel).

## In scope

- **Services completeness** (`systui-ui`): surface the existing full-unit-list collector
  behind `ALL / RUNNING / INACTIVE / ENABLED / FAILED` filters; add enabled/disabled
  state via `systemctl list-unit-files`. The failed-only fast path stays the default
  view; the full list is gathered in the tiered/slow refresh (v0.8.3 machinery).
- **Connectivity tests panel** (`systui-ui` + `connectivity.rs`): wire the existing
  ping / DNS / TCP probes into the Network screen as **on-demand** diagnostics against
  sensible default targets (default gateway, configured DNS servers, inventory hosts);
  results are reachability only, never mutations.
- **Firewall collector** (`systui-collectors/firewall.rs`): detect the active backend
  and summarise tables / chains / rule counts; flag rules with no bound process and
  rules conflicting with a listening port. Read-only.
- **Docker compose + image-hygiene collectors** (`systui-collectors/docker.rs`):
  compose-project discovery and image-store summary (count, total size, dangling, last
  prune). Both read-only; render the prototype's two bottom Docker panels.
- **Docker prune mutation** (`systui-actions/docker.rs`): `prune dangling` through the
  action engine (risk → preview → confirm → execute → audit; honoured by read-only).
- **Packages/updates collector** (`systui-collectors/packages.rs`): pending + security
  update counts across apt / dnf / pacman / zypper, feeding the Dashboard **UPDATES**
  at-a-glance tile (and a small Packages view if the layout allows). Read-only; never
  triggers a package-manager refresh that writes lock files unexpectedly.
- **Anacron source + cron "run now"** (`systui-collectors/cron.rs`,
  `systui-actions/cron.rs`): parse `/etc/anacrontab` into the existing cron model;
  add a `run now` action for a user-crontab entry through the engine.
- **Persistence store** (`systui-storage`): a small local store (under the existing
  config/state dir) holding (a) periodic **health-score + finding-count snapshots**
  per host to power the Dashboard and Security **trend** lines, (b) per-host **session
  notes**, and (c) **saved log searches**. Schema versioned; writes are best-effort and
  never block a refresh.

## Out of scope (deferred)

- **Finding lifecycle states** (`accept` / `ignore` / `false-positive` / `fixed`):
  these belong to **phase 9 (v0.9 Policies)** §S9.4 — not duplicated here. The Security
  screen keeps showing evidence-based findings without state buttons.
- **"apply fix" auto-remediation** (the green `apply` button): auto-remediation is
  **out of scope until v1.1+** per `ROADMAP.md`. No one-click fixes this phase. (A
  read-only **`copy fix`** that copies an existing recommendation string is *not*
  included either — deferred with the rest of the finding-action row to avoid a
  half-built actions column.)
- **"Backups detected" tile** and **per-source log byte-rates**: not measurable
  honestly with what we collect (backup artifacts are heuristic; byte-rates need
  sustained sampling we do not do). Omitted rather than faked — consistent with the
  v0.8.2 contract.
- **Command palette (Ctrl-K) / fuzzy search**: still deferred (a separate feature,
  carried over from v0.8.2 out-of-scope).
- **New visual identity / theme work**: none. Colours stay in `theme.rs` tokens; this
  phase only adds widgets to existing screen layouts.
- **Policies / expected-state**: phase 9, unchanged.

## Key decisions

- **Parity with the prototype, still only with real data.** Same contract as v0.8.2:
  every new panel is backed by a real collector, an existing diagnostic, or a real
  stored value. Anything not honestly measurable stays omitted. See
  [[feedback-ui-real-data]].
- **Wire before you collect.** Where data already exists (full unit list, connectivity
  probes), this phase *surfaces* it rather than re-collecting it. New collectors are
  added only for genuinely missing data (firewall, compose, image hygiene, anacron,
  packages).
- **Respect v0.8.3 refresh tiering.** New slow-changing collectors (packages, image
  hygiene, full unit list, firewall) go in the **slow tier**, run concurrently, and are
  cancellable with per-collector timeouts — they must not reintroduce a stall or undo
  the optimization phase. Connectivity tests are **on-demand only**, never on the tick.
- **Persistence is additive and best-effort.** The new store lives beside the existing
  config/audit data, is schema-versioned, and a write failure degrades to "no trend /
  no saved notes", never a crash or a blocked refresh. Trends start empty and fill over
  time (like the v0.8.2 sparklines).
- **Mutations go through the engine.** `prune dangling` and cron `run now` use the
  existing action pipeline (risk → preview → confirm → execute → audit) and are blocked
  in read-only mode. No new bypass path. See [[feedback-workflow-versiones]].
- **Render stays a pure function of `App`.** All new panels read from `App`; the
  `TestBackend` render tests keep working. No inline `Color::Rgb` — tokens only.

## Sessions

- **S8e.1 — Context** *(this file)* + ROADMAP insert for v0.8.4.
- **S8e.2 — Services & Connectivity wiring**: surface the full unit list + enabled state
  behind the `ALL/RUNNING/INACTIVE/ENABLED/FAILED` filters; wire the existing
  connectivity probes into the Network **Connectivity tests** panel (on-demand).
  **Done.** Added a `UnitFilesCollector` (`systemctl list-unit-files`) for the
  enabled/disabled state `list-units` lacks, and a `gather_services` group that
  collects the full unit list + enabled set concurrently (degrades to empty,
  bounded by the per-collector timeout — no stall, v0.8.3 preserved). The
  Services screen gained a **filter chip bar** (`f` cycles ALL/FAILED/RUNNING/
  INACTIVE/ENABLED) over the existing two-column table+detail, with a per-row
  state dot, an enabled marker and an enabled field in the detail; the live
  `--failed` fast path still backs the FAILED view and the dashboard/health.
  Network gained an on-demand **Connectivity tests** panel: `c` runs short pings
  against the host's own gateway + configured DNS servers (deliberately *not*
  arbitrary external hosts) in a **background task** (its own channel, mirroring
  the refresh path) so the UI never freezes during the probes. Status-bar hints
  and the help overlay updated; render tests + parser/target unit tests added;
  gates green.
- **S8e.3 — Firewall**: `firewall.rs` collector (backend + tables/chains/rules +
  conflict notes) → the Network **Firewall** panel.
  **Done.** New `FirewallCollector` names the active manager (`firewalld` via
  `firewall-cmd --state`, `ufw` via `ufw status`) and reads the *effective*
  ruleset — `nft list ruleset` first, then `iptables -S` — into real table,
  chain and rule-statement counts (text parsers, fixture-tested). All probes are
  read-only and marked `privileged`; when the listing is denied/unavailable the
  snapshot degrades to a "needs privilege?" note instead of claiming "no
  firewall". It collects concurrently with the security scan inside the existing
  `network_group` (bounded by its timeout — no stall). The Network right rail
  gained a **Firewall** panel (backend + state, tables, chains, `N active`
  rules, caveat notes). The prototype's "rule but no process bound / conflicts
  with a listening port" correlation is **deferred** — it cannot be derived
  reliably across backends without fragile, fakeable rule parsing, so per the
  real-data-only contract it is left out rather than guessed. Render + parser
  tests added; gates green.
- **S8e.4 — Docker compose + image hygiene + prune**: the two omitted Docker panels
  from new collectors; `prune dangling` mutation through the action engine.
  **Done.** Added `compose_projects` (`docker compose ls -a --format json` →
  name, status, config file, service count) and `image_hygiene` (`docker system
  df` totals + a `docker images --filter dangling=true` count) collectors,
  fixture-tested. They collect concurrently with `container_stats` inside the
  existing docker group (which was refactored from a 5-tuple to a `DockerGather`
  struct for readability), gated on Docker being available, and degrade to empty
  (the Compose plugin may be absent). The Docker screen grew a third row with
  **Compose projects** and **Image hygiene** panels alongside the existing
  table/risks/detail. A new host-scoped **`DockerPruneAction`** (`docker image
  prune -f`, Medium risk, reports reclaimed space) runs through the action
  engine; `p` on the Docker tab triggers it, blocked in read-only mode and
  audited like every other mutation. Status-bar/help hints, render tests and
  action/parser unit tests added; gates green.
- **S8e.5 — Packages & Anacron**: packages/updates collector → Dashboard **UPDATES**
  tile (+ Packages view if it fits); anacron source in Crons; cron **run now** action.
  **Done.** New `PackagesCollector` detects the package manager and counts
  pending updates **cache-only** (`apt list --upgradable`, `dnf --cacheonly
  check-update`, `pacman -Qu`, `zypper list-updates`) — never refreshing
  metadata or taking a lock; apt also yields a security-pocket count, others
  report 0 rather than guessing. It joins the concurrent gather (`gather_packages`,
  own timeout) and feeds a new at-a-glance **UPDATES** tile (pending, security
  highlighted; "n/a" when no manager). `/etc/anacrontab` is now parsed into
  `CronEntry`s (`CronSource::Anacron`, period→`@daily`/`@weekly`/`@monthly`
  mapping, job-id stripped) and shown in the Crons table; they stay read-only
  (only `CronSource::User` entries are editable). A new `CronOp::RunNow` runs a
  user job's command immediately via `sh -c` through the action engine (High
  risk → typed confirmation, no crontab change), triggered by `n` on the Crons
  tab and blocked in read-only. Status-bar/help hints, render tests and
  parser/action unit tests added; gates green. A dedicated Packages *view* was
  not needed — the UPDATES tile covers the prototype's at-a-glance cell.
- **S8e.6 — Persistence-backed panels**: the local store for health/finding-count
  snapshots (Dashboard + Security **trend** lines), per-host **Session notes**, and
  **Saved searches** in Logs.
  **Done.** New `systui-storage::store` — a single versioned JSON document
  (`state.json`, schema v1, best-effort: a missing/corrupt file loads as default,
  a write failure never blocks the UI) holding per-host daily health/finding
  **snapshots** (deduped per day, capped), per-host **session notes**, and
  **saved log searches** (deduped, most-recent first). The TUI loads it at
  startup and flushes on exit; each refresh records today's snapshot. The
  **Dashboard** health panel now shows `was N 7d ago` and the **Security** header
  shows `↓N from last week`, both computed from the nearest past snapshot
  (`baseline`) and empty until history accumulates (like the sparklines). A
  **Session notes** panel was added to the Dashboard right rail with an `n`
  single-line note overlay; a **Saved searches** panel in the Logs right rail
  with `S` to save the current query and ↑/↓ + Enter to apply one. Status-bar
  and help hints updated; store + render tests added (one pre-existing dashboard
  render test was bumped to a taller viewport, since the extra panel needs the
  prototype's height). Gates green.
- **S8e.7 — Polish & close**: render-test refresh, help-overlay/keymap updates for the
  new actions, a final prototype-vs-app panel inventory; gates green; merge `--no-ff`
  into `main` + tag `v0.8.4`.

## Definition of Done

- The panels the v0.8.2 reskin omitted are now present and **backed by real data**:
  Services filters, Network Connectivity tests + Firewall, Docker Compose + Image
  hygiene, the Dashboard UPDATES tile, anacron in Crons, and the persistence-backed
  trend / session-notes / saved-searches panels.
- The only panels still absent are the explicitly deferred ones (finding-state buttons,
  apply-fix, backups, per-source byte-rates, command palette), each documented as out
  of scope with a reason.
- `prune dangling` and cron `run now` run through the action engine and are blocked in
  read-only mode and audited.
- New slow collectors run in the tiered/concurrent refresh with timeouts; the TUI never
  stalls (v0.8.3 guarantee preserved); connectivity tests are on-demand only.
- The persistence store is schema-versioned; a write/read failure degrades gracefully.
- Visual identity, theme tokens, chrome, keymaps elsewhere and the safety model are
  unchanged; render stays a pure function of `App`; `cargo fmt --check`,
  `cargo clippy -D warnings` and `cargo test --workspace` pass.

## Risks & open questions

- **Cross-distro package counts**: apt/dnf/pacman/zypper differ in how cheaply "pending
  updates" can be read without a (slow, lock-taking) metadata refresh. Decision: read
  cached state only; if a manager needs a refresh to answer, show "unknown" rather than
  forcing one. Keep it in the slow tier.
- **Firewall privilege**: listing nftables/iptables rules often needs root. Honour the
  exec-mode model — show what the current privilege allows and label the rest as
  "needs privilege", never silently empty.
- **Connectivity target choice**: probing arbitrary hosts could surprise users.
  Restrict default targets to gateway / configured DNS / inventory hosts, and keep it
  on-demand (a keypress), never automatic.
- **Compose discovery cost**: `docker compose ls` / label scanning can be slow on big
  hosts. Slow tier + timeout; partial data on timeout.
- **Persistence growth**: snapshots accumulate. Cap retention (e.g. rolling window) so
  the store stays small; document the schema and the window.
- **Render-test churn**: new panels change `TestBackend` snapshots — update assertions
  on content, not exact cell positions, where possible (same discipline as v0.8.2).
