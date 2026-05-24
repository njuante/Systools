# Phase 8.6 — v0.8.2 UI redesign

> First session deliverable. Freezes scope and decisions for v0.8.2 before coding.
> An **intermediate** phase between v0.8.1 (management & UX polish) and v0.9 (Policies).
> See [`../ROADMAP.md`](../ROADMAP.md) and [`../METHODOLOGY.md`](../METHODOLOGY.md).
> Built on `release/v0.8.2` (from `main` after v0.8.1); tag `v0.8.2` when the DoD is met.

## Goal

Adopt the **approved visual design** in [`../interfaz/`](../interfaz/) — a polished,
truecolor, host-centric TUI generated as a Ratatui spec (`SysTUI Ratatui Spec.html`)
with reference screenshots. v0.8.1 already made the app functional and lightly
polished; v0.8.2 makes it **look like a finished product**: a single-source-of-truth
truecolor theme, a richer 3-line chrome (host-attached bar + health gauge + mode
badge), numbered tabs with count badges, and multi-panel screens that match the
prototype.

This is a **visual/structural** phase: it reskins and re-lays-out the existing screens
and reuses all current data, collectors and the action engine unchanged. No new
collectors, no new mutations, no behaviour changes to the safety model.

The design intent is captured by the prototype:

- **Theme tokens** (`SysTUI Ratatui Spec.html` §13): a single `Theme` in
  `systui-ui/theme.rs` holding `Color::Rgb(...)` values — `bg`, `fg`, `fg_strong`,
  `fg_muted`, `fg_dim`, `accent`, `critical`, `high`, `medium`, `low` — and **no
  inline `Color::Rgb` anywhere else**.
- **Frame skeleton** (§13): `top_bar` (3 rows) · `tabs` (1 row) · `body` (min) ·
  `status_bar` (1 row); overlays (palette, confirm, toasts) render last.
- **Per-screen widget recipes** (§14) for Hosts, Dashboard, Services, Logs, Network,
  Crons, Security, Docker.

## In scope

- **Theme overhaul** (`systui-ui/theme.rs`): replace the 16-colour ANSI palette with
  the spec's truecolor tokens. Keep the legacy field names as aliases mapped onto the
  new tokens so un-migrated screens keep compiling; migrate screens to the canonical
  token names as they are reworked.
- **Chrome** (`systui-ui/ui.rs`): a 3-row top bar — brand + **host-attached pill**
  (status dot, `user@host`, transport/latency/mode) + a **health `LineGauge`** + a
  **mode badge** (READ-ONLY / SAFE / PRIVILEGED); a tab row with **numbered tabs and
  count badges** (e.g. `4 Services 2`); and a status-bar footer with contextual key
  hints and a refresh/follow indicator.
- **Dashboard** (this session): the multi-panel layout — four metric tiles
  (CPU/RAM/DISK/LOAD with `Sparkline`s), a **health-score panel** with the deduction
  breakdown and a hand-drawn gradient bar, a **critical-findings list** with
  severity-coloured left edge bars, and an **at-a-glance** grid. Adds small CPU/RAM
  history ring buffers to feed the sparklines.
- **Subsequent screens** (later sessions): Services, Logs, Network, Crons, Security,
  Docker re-laid-out to their §14 recipes; the **Hosts** screen (fleet) restyled to
  the card-grid prototype.

## Out of scope (deferred)

- **Command palette (Ctrl-K)** and **nucleo/fzf fuzzy search** (§15): a strong
  candidate, but a separate feature; not required to land the visual identity. Tracked
  for a later session/phase.
- **Mouse hit-testing** (clickable at-a-glance cells, etc.): keyboard-first stays the
  contract for now.
- **Theme switching / light theme**: only the `dark_green` theme ships; the struct is
  shaped so more themes are a later addition.
- **New data / collectors**: no new probes. Where the prototype shows data we do not
  yet collect (e.g. per-source log rates, image hygiene totals), render what we have
  and leave the rest for the relevant feature phase — never fake values.
- **Policies / expected-state** — phase 9 (v0.9), unchanged.

## Key decisions

- **One theme, one source of truth.** All colour lives in `theme.rs` as `Color::Rgb`
  tokens; screens reference `app.theme.*` only. This is enforced by code review, not a
  lint, this phase. The legacy aliases are a **migration bridge**, not a parallel
  palette — they resolve to the same RGB values and shrink as screens move to the
  canonical names.
- **Reskin, don't rewrite the engine.** The redesign is confined to `systui-ui`
  (`theme.rs`, `ui.rs`, `fleet.rs`). Collectors, actions, security and storage are
  untouched. Render stays a **pure function of `App`**, so the `TestBackend` tests
  keep working.
- **Truecolor target, graceful on 256-colour.** The design assumes a truecolor
  terminal (the prototype header says `truecolor`); ratatui downsamples `Rgb` on
  lesser terminals, which is acceptable. No separate 16-colour theme is maintained.
- **Match the prototype, but only with real data.** Layouts follow the screenshots;
  values come from existing collectors. Panels for not-yet-collected data are omitted
  or shown empty rather than mocked.
- **History for sparklines is UI-local.** CPU/RAM history is a short ring buffer on
  `App`, pushed on each refresh; it starts empty and fills over time. No collector or
  storage change.

## Sessions

- **S8c.1 — Context** *(this file)* + ROADMAP insert for v0.8.2 + **foundation**:
  theme tokens, the 3-row chrome (top bar / tabs-with-badges / status bar), and the
  **Dashboard** rebuilt to its multi-panel recipe (metric tiles + health score +
  findings + at-a-glance), with CPU/RAM history for the sparklines.
- **S8c.2 — Services & Docker**: unit/container tables with severity dots + detail
  panes and the risk-check side panel (§14 Services/Docker recipe).
- **S8c.3 — Logs**: live-tail widget with level badges, the error-fingerprint side
  panel and the follow/pause indicator.
- **S8c.4 — Network & Crons**: exposure table (address colour-coding), interfaces /
  firewall / connectivity panels; cron table with severity left-bar + schedule
  preview + backup callout.
- **S8c.5 — Security & Hosts**: security score header + evidence-block findings; the
  Hosts/fleet card grid with health bars and tag chips.
- **S8c.6 — Polish & close**: spacing/alignment pass, help overlay, render-test
  refresh; final gates; merge `--no-ff` into `main` + tag `v0.8.2`.

## Definition of Done

- The TUI matches the approved prototype's **visual identity**: truecolor theme,
  3-row chrome with host-attached bar + health gauge + mode badge, numbered tabs with
  count badges, and multi-panel screens.
- All colour comes from `theme.rs` tokens; there are no inline `Color::Rgb` calls in
  screen code.
- Every screen from the prototype is rendered with **real data**; no mocked values.
- Behaviour, keymaps and the safety model are unchanged; the render is still a pure
  function of `App` and the `TestBackend` tests pass.
- `cargo fmt --check`, `cargo clippy -D warnings` and `cargo test --workspace` pass.

## Risks & open questions

- **Truecolor assumption**: on non-truecolor terminals colours downsample and may look
  muddier. Accepted; the tokens are chosen to remain legible after downsampling.
- **Render-test churn**: re-laying-out screens changes the `TestBackend` snapshots /
  assertions. Update them deliberately and keep assertions on content, not exact cell
  positions, where possible.
- **Scope creep into features**: the prototype shows data we do not collect (palette,
  per-source log rates, image hygiene). Resist implementing those here — render what
  exists, defer the rest, never fake.
- **Theme bloat**: the legacy-alias bridge can linger. Treat it as debt to retire as
  each screen migrates to canonical token names.
- **Sparkline emptiness**: history starts empty, so tiles look flat on first frames.
  Acceptable; it fills within seconds of the refresh cadence.
