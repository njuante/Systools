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
- **S11.2 — Cockpit dashboard**: per-domain status-card grid with one-line verdicts.
- **S11.3 — Drill-down pilot**: the summary-band → raw-detail pattern and the dense-mode
  toggle on Services, Logs and System.
- **S11.4 — Drill-down rollout**: the remaining tabs (Network, Docker, Crons, Databases,
  Security, Processes).
- **S11.5 — Logs aggregation-first + JSON export**.

## Definition of done

- All five sessions landed; `cargo build`, `cargo clippy --all-targets` and `cargo test`
  green; `TestBackend` snapshots updated for the new layouts.
- The cockpit, drill-down + dense toggle, the Sober↔Rich style switch (persisting across
  restarts) and JSON log export all work against a real local host.
- Tagged `v1.1.0`; `release.yml` artifacts produced as for v1.0.0.
