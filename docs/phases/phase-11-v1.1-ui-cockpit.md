# Phase 11 — v1.1 UI replanteamiento: cockpit + drill-down

> First session deliverable. Freezes scope and decisions for v1.1 before coding.
> The first **post-1.0** phase. See [`../ROADMAP.md`](../ROADMAP.md) and
> [`../METHODOLOGY.md`](../METHODOLOGY.md).
> Built on `release/v1.1` (from `main` after v1.0.0); tag `v1.1.0` when the DoD is met.

## Goal

Rethink the interface so a **status read is more than a wall of text**. Today the app is
ten tabs, each with four-to-six dense `Table`/`Paragraph` panels, and the only real
"visual" element is a single CPU/RAM `Sparkline`. v1.1 makes the app **cleaner and more
visual**: a landing **cockpit** of per-domain status cards with one-line verdicts,
**progressive disclosure** (summary first, raw table/detail behind a keystroke), a
**selectable visual style** (sober now, rich later), and **JSON export** for logs /
incidents so failures can be captured fast.

This is a **visual/structural** phase: it re-lays-out and reskins existing screens and
reuses all current collectors, the action engine and the safety model unchanged. No new
collectors, no new mutations.

### Direction change from v0.8.2

The v0.8.2 redesign was driven by "match the approved prototype in `docs/interfaz/`".
That layout-matching constraint is **dropped** for v1.1 — the user asked to "forget the
model we were copying" and design a cleaner, more visual interface from first
principles. The **only-real-data rule still holds**: where we don't collect the data, we
omit the panel rather than mock it.

## In scope

- **Visual style selector** (`systui-ui`): a new `VisualStyle { Sober, Rich }` enum
  mirroring `ThemeKind` (`ALL`, `next()`, `config_name()`, `from_config_name()`,
  `label()`). Persisted via `[general] visual_style` (default `sober`). **Sober** ships
  fully; **Rich** (braille/canvas high-res meters) is architected now and filled
  incrementally — it may start as a thin wrapper that reuses Sober.
- **Shared sober widget vocabulary** (`systui-ui`): reusable pure-of-`Theme` helpers —
  status chip, severity dot, progress bar (reusing `gauge_bar`), labelled sparkline,
  one-line verdict, KPI tile. No inline `Color::Rgb`.
- **Cockpit dashboard**: `render_dashboard` becomes a per-domain status-card grid
  (System / Services / Network / Docker / Security / Logs / Crons / Databases). Each
  card: domain accent, one mini-visual, and a one-line verdict derived from
  `app.health` / `app.findings` / existing snapshots. **No tables on the landing.**
- **Drill-down across all 10 tabs**: each tab leads with a compact visual summary band;
  the raw table/detail sits behind `Enter`/expand. A **dense-mode / expand-all toggle**
  restores the power-user dense view.
- **Logs aggregation-first + JSON export**: default view is error fingerprints (already
  computed client-side) plus a severity mini-timeline; the raw tail is opt-in. An export
  action serialises the current logs view (entries + active filter + fingerprints + host
  + timestamp) to **JSON** into a `systui-storage` exports dir, toasting the path.

## Out of scope (deferred)

- **Rich style full build-out**: the selector and Sober ship now; rich braille/canvas
  meters land incrementally, not all in the first session.
- **New collectors / data**: no new probes; render what we collect, omit the rest.
- **Non-JSON export formats** (markdown/plain): JSON only for v1.1.
- **Mouse hit-testing**: keyboard-first stays the contract.

## Key decisions

- **Reskin, don't rewrite the engine.** Confined to `systui-ui` plus small additive
  config fields in `systui-core`/`systui-storage`. Collectors, actions, security
  untouched. **Render stays a pure function of `App`** so `TestBackend` tests keep
  working.
- **Two selectable styles, sober is the default.** The style selector mirrors the proven
  `ThemeKind` pattern and persists like the theme. Rich is optional and incremental.
- **Cockpit over data dump.** The landing answers "is this machine OK?" at a glance;
  detail is one keystroke away. Verdicts are derived from real findings/health/snapshots.
- **Aggregation over raw logs.** Logs default to fingerprints + a severity timeline; the
  raw tail is opt-in. Export captures the current view as JSON for incident handoff.
- **Only real data.** Any panel we can't back with a collector is omitted, never mocked.

## Sessions

- **S11.1 — Context** *(this file)* + `release/v1.1` branch + **foundation**: the
  `VisualStyle` selector (enum + config persistence + cycle keybinding + tests) and the
  shared sober widget vocabulary. No large visual change yet.
  **Done.** `VisualStyle { Sober, Rich }` (`visual_style.rs`) mirrors `ThemeKind`,
  persists in `[general] visual_style`, cycles with `V`, and shows in the status bar
  next to the theme. Branched on `release/v1.1` (from `feat/tui-polish`, which carries
  the multi-theme/cron-builder work not yet in `main`).
- **S11.2 — Cockpit dashboard**: per-domain status-card grid with one-line verdicts.
  **Done.** A new `widgets.rs` provides `StatusLevel`, a `status_card` and a `grid`
  helper. `render_dashboard` now shows the vital tiles (CPU/RAM/DISK/LOAD) over a
  **Status** panel of nine accented cards (Services, Network, Docker, Security, Logs,
  Crons, Databases, System, Updates), each with a status dot and a plain-language
  verdict from real data; a right rail keeps the health score, Top findings and session
  notes. The dense `at_a_glance` text grid was removed.
- **S11.3 — Drill-down pilot**: the summary-band → raw-detail pattern and the dense-mode
  toggle on Services, Logs and System.
  **Done.** Added a session-local dense mode (`Shift+D`, `App::dense`). The rule across
  screens: **clean by default shows the primary panel(s) full-width; dense restores the
  full multi-panel layout.** Pilots: Services (table → +detail pane), System (identity +
  memory → +disks +users), Logs (live tail → +fingerprints/sources/saved-searches). The
  cockpit cards also reveal their secondary breakdown line in dense.
- **S11.4 — Drill-down rollout**: the remaining tabs (Network, Docker, Crons, Databases,
  Security, Processes).
  **Done.** Same rule applied to Processes (list → +detail), Network (exposure map →
  +connectivity/interfaces/DNS/connections/firewall), Docker (container table → +risks/
  detail/compose/hygiene), Crons (jobs table → +preview/timers/summary) and Databases
  (instances → +operational detail). Security keeps its header + findings layout (it is
  already a summary + list with no secondary rail to hide).
- **S11.5 — Logs aggregation-first + JSON export**.
  **Done (with a scope adjustment).** Shipped the **JSON export**: `e` on the Logs tab
  writes the current view to `~/.local/share/systui/exports/systui-logs-<host>-<ts>.json`
  (host, timestamp, active filter, an error-fingerprint aggregation and the raw entries)
  off the render path via an `export_requested` flag; a transient status line reports the
  path. **Adjustment:** the planned "invert Logs to aggregation-first by default" was
  dropped — the S11.3/S11.4 drill-down already makes the live tail the clean default and
  surfaces the fingerprints/sources rail under dense (`D`), so inverting the default would
  contradict the now-consistent per-tab rule. The fingerprint aggregation instead lives in
  the export payload, where it directly serves "grab a failing host's logs fast".

## Definition of done

- All five sessions landed; `cargo build`, `cargo clippy --all-targets` and `cargo test`
  green; `TestBackend` snapshots updated for the new layouts.
- The cockpit, drill-down + dense toggle, the Sober↔Rich style switch (persisting across
  restarts) and JSON log export all work against a real local host.
- Tagged `v1.1.0`; `release.yml` artifacts produced as for v1.0.0.
