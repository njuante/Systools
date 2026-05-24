# Phase 8.7 — v0.8.3 Optimization

> First session deliverable. Freezes scope and decisions for v0.8.3 before coding.
> An **intermediate** phase between v0.8.2 (UI redesign) and v0.9 (Policies).
> See [`../ROADMAP.md`](../ROADMAP.md) and [`../METHODOLOGY.md`](../METHODOLOGY.md).
> Built on `release/v0.8.3` (from `main` after v0.8.2); tag `v0.8.3` when the DoD is met.

## Goal

Make SysTUI **fast and fluid**, locally and especially over SSH. The v0.8.2
redesign exposed real performance problems once the app was used against remote
hosts; v0.8.3 is a **pure performance/optimization phase** — no new features, no
visual changes. The headline outcome: the TUI never freezes, refreshes happen in
the background, and a remote session feels as responsive as a local one.

This phase changes *how* work is scheduled and executed, not *what* the app shows.
Behaviour, screens and the safety model stay identical; the `TestBackend` render
tests and the action-engine contract must keep passing.

## The problem (measured in v0.8.2)

- **Refresh is synchronous on the UI thread.** `systui-ui::event_loop`
  (`crates/systui-ui/src/lib.rs`) calls `data::refresh_blocking`
  (`crates/systui-ui/src/data.rs`) inline: it `runtime.block_on(...)`s the whole
  gather, so during a refresh the loop does not draw or read input — the UI is
  frozen for the gather's duration. Auto-refresh fires on a timer, so it freezes
  periodically.
- **Collectors run sequentially.** `refresh_blocking` runs the system report, then
  network+security, then docker+crons one after another; each is itself a series of
  awaited commands. Nothing that is independent runs concurrently.
- **Every refresh re-collects everything**, including data that rarely changes (OS,
  kernel, hostname, capabilities, interface list).
- **SSH latency** was the multiplier: fixed in v0.8.2 with connection multiplexing
  (`ControlMaster`/`ControlPersist` in `crates/systui-transport/src/ssh.rs`), which
  took a full remote gather from many seconds to ~1s. That fix is the *floor*;
  this phase removes the remaining stalls above it.

## In scope

- **Asynchronous / background refresh.** Move the gather off the UI thread: a
  worker (thread or tokio task) runs the collectors and posts results to the app
  via a channel; the event loop keeps drawing and handling input, showing a subtle
  "refreshing…" indicator. A refresh in flight never blocks input; results swap in
  atomically when ready. Manual `r` and auto-refresh both go through this path.
- **Concurrent collectors.** Run independent collectors concurrently within a
  refresh (system / network / docker / logs / databases), bounded sensibly, instead
  of strictly sequentially. Preserve ordering only where a real dependency exists
  (e.g. exposure map needs the network snapshot; docker/cron findings append to the
  shared list).
- **Tiered refresh / caching.** Separate **slow-changing** data (OS, kernel,
  hostname, capabilities, interfaces) from **live** data (CPU/RAM/load, processes,
  logs, connections). Collect the slow set once (or rarely) and the live set on each
  refresh, so a tick does less work.
- **Command-count reduction.** Where a collector issues many small commands, batch
  reads (e.g. one `cat` of several `/proc` files, fewer `systemctl`/`ss`
  invocations) to cut round-trips — this compounds with SSH multiplexing.
- **Cancellation & timeouts.** A slow/hung host must not wedge the UI: an in-flight
  refresh is cancellable and per-collector timeouts degrade to partial data, never a
  stall. (The fleet already bounds per-host review; bring the same discipline to the
  single-host refresh.)
- **Build/runtime profile.** Tune the release profile (LTO, codegen-units,
  `opt-level`, `panic=abort` if acceptable) for a smaller, faster binary and quicker
  startup; verify with measurements.
- **Measurement harness.** Add lightweight timing (behind a flag/log) so refresh
  duration and per-collector cost are observable, and capture before/after numbers
  in the phase notes — optimization without measurement is guesswork.

## Out of scope (deferred)

- **Any UI/visual change** beyond a refresh indicator/spinner. Layouts, colors and
  screens are frozen from v0.8.2.
- **New collectors, actions or data.** This is performance only.
- **Rewriting the SSH transport** (e.g. embedding a Rust SSH library). The
  system-`ssh` + multiplexing approach stays; we optimize around it.
- **Policies / expected state** — that is phase 9 (v0.9), unchanged.

## Key decisions

- **Keep render a pure function of `App`.** The async refresh updates `App` from the
  worker's results on the main thread (drain the channel in the loop, then draw), so
  rendering stays synchronous and `TestBackend`-testable. No locking in the renderer.
- **Single shared tokio runtime.** Reuse the existing runtime; spawn the background
  gather as a task and communicate results over a channel (`std::sync::mpsc` or
  `tokio::sync::mpsc`), rather than `block_on` inline.
- **Correctness over raw speed.** Concurrency must not change results or reorder
  findings in a user-visible way; the merged findings list stays worst-first and
  deterministic. Partial data on timeout is explicit (existing `ViewState`).
- **Measure, then optimize.** Land the timing harness first; justify each change
  with a before/after number in the phase notes.

## Sessions

- **S8d.1 — Context** *(this file)* + ROADMAP insert + a **measurement harness**
  (timing of refresh and per-collector cost, behind `SYSTUI_LOG`/a flag) and
  captured baselines (local and over SSH).
- **S8d.2 — Background refresh**: move the gather off the UI thread (worker + channel),
  keep the loop responsive with a refresh indicator; manual + auto refresh through it.
- **S8d.3 — Concurrent collectors**: gather independent collectors concurrently within
  a refresh, preserving real dependencies and deterministic output.
- **S8d.4 — Tiered refresh + caching**: split slow-changing vs live data; collect the
  slow set rarely; reduce per-tick work.
- **S8d.5 — Command batching + cancellation/timeouts**: cut round-trips; make an
  in-flight refresh cancellable with per-collector timeouts.
- **S8d.6 — Build profile + close**: release-profile tuning, final before/after
  numbers, gates; merge `--no-ff` into `main` + tag `v0.8.3`.

## Definition of Done

- The TUI **never freezes** on refresh (local or SSH): input and redraw stay live
  while a gather runs in the background; a clear indicator shows refresh activity.
- Independent collectors run **concurrently**; slow-changing data is **not**
  re-collected every tick.
- A slow/unreachable host **degrades to partial data on a timeout** without stalling
  the UI; an in-flight refresh is cancellable.
- **Before/after measurements** are recorded (refresh latency local and over SSH,
  startup time, binary size) showing the improvement.
- Behaviour, screens, keymaps and the safety model are unchanged; render stays a pure
  function of `App`; `cargo fmt --check`, `cargo clippy -D warnings` and
  `cargo test --workspace` pass.

## Risks & open questions

- **Concurrency races / reordering**: parallel collectors must produce the same,
  deterministic `App` state. Keep the merge/sort centralised and tested.
- **Borrow/lifetime friction** moving the gather to a worker (the transport and
  config must be shareable). Prefer `Arc`-shared, `Send` transports; the SSH and
  local transports are stateless, which helps.
- **Stale-while-refreshing UX**: showing the previous data with a spinner is good;
  make sure a failed background refresh surfaces the error without wiping good data.
- **Cancellation correctness**: cancelling mid-gather must not leave half-applied
  state — only swap in a complete result.
- **`panic=abort` vs unwind**: only adopt if it doesn't break tests or needed
  behaviour; measure the binary-size/startup win before committing to it.
- **Don't micro-optimize the renderer**: ratatui re-renders the whole frame cheaply;
  the wins are in scheduling and I/O, not in the draw path.

## Progress

### S8d.1 — Context + measurement harness ✅

- Context file (this) + ROADMAP insert: done.
- **Measurement harness**: every collector and the overall gather are wrapped in
  `systui_collectors::timing::timed`, which emits how long the work took as a
  `tracing` event under the `systui::perf` target. Wired into both refresh paths —
  the TUI refresh (`systui-ui::data`, `refresh_total`) and the headless gather
  (`systui-report::gather`, `gather_total`, the non-TUI equivalent of the dashboard
  refresh). Dormant unless enabled. Capture with:

  ```sh
  SYSTUI_LOG=systui::perf=info systui report -o /dev/null            # local, full breakdown
  SYSTUI_LOG=systui::perf=info systui report --host <id> -o /dev/null # over SSH
  SYSTUI_LOG=systui::perf=info systui                                 # live TUI refresh
  ```

#### Baselines (release build, sequential gather)

Per-collector cost in ms, captured via `systui report` (headless gather, identical
collector set and order to the dashboard refresh).

| collector      | local (ms) | SSH `vpn` (ms) |
|----------------|-----------:|---------------:|
| system         |      210.8 |          298.7 |
| processes      |       20.8 |           15.0 |
| failed_units   |        7.6 |           10.1 |
| logs           |       26.3 |           23.4 |
| network        |       36.6 |           42.5 |
| security_scan  |       28.5 |          310.5 |
| docker         |       14.2 |            7.8 |
| crons          |        0.2 |           91.7 |
| cron_findings  |        0.0 |           11.5 |
| timers         |        9.2 |           22.9 |
| databases      |       36.0 |           68.2 |
| **gather_total** | **390.6** |      **902.5** |

**Reading the numbers.** Locally the `system` collector alone is ~54% of the total
because it samples CPU with a ~200ms delay between two `/proc/stat` reads — a fixed
floor unrelated to I/O. Over SSH the per-command latency multiplier shows up exactly
where a collector issues many small commands: `security_scan` (28→310ms), `crons`
(0.2→92ms), `cron_findings`, `databases`. The gather is strictly sequential, so the
total is the sum of all of them — which is what S8d.2 (background), S8d.3
(concurrency) and S8d.5 (batching) attack. Slow-changing data folded into `system`
(hostname/kernel/etc.) is re-collected every tick → S8d.4 (tiering).

### S8d.2 — Background refresh ✅

The single-host refresh no longer blocks the UI thread. The gather logic moved out
of `refresh_blocking` into a pure `systui-ui::data::gather` that returns a
`RefreshResult` (a self-contained bundle of the whole refresh) and touches no `App`
state. The event loop (`systui-ui::lib`) spawns it as a task on the shared tokio
runtime, posts the result over an `std::sync::mpsc` channel, and drains the channel
each tick via `try_recv`, folding the result in with `apply_refresh` on the main
thread. Rendering therefore stays a pure, synchronous function of `App` — no locking
in the draw path.

Key properties:

- **Never freezes.** Input and redraw stay live for the whole gather; `q`, tab
  navigation and scrolling respond even mid-refresh (previously a `block_on` wedged
  the loop for the gather's full duration). Manual `r` and auto-refresh both go
  through the background path.
- **Coalesced.** `spawn_refresh` is a no-op while a gather is in flight (`App.refreshing`),
  so requests never stack up — at most one gather runs at a time.
- **Atomic swap, error-safe.** Only a finished `RefreshResult` is applied; a failed
  background refresh surfaces the error via `ViewState` but leaves the previous good
  data on screen (covered by `failed_refresh_keeps_previous_good_data`).
- **Indicator.** A subtle `⟳ refreshing` marker in the top bar shows activity while a
  gather is in flight.
- **Transport sharing.** `run` converts the `Box<dyn Transport>` to
  `Arc<dyn Transport>` (the trait is `Send + Sync`) so the gather task owns a clone;
  the main thread keeps one for synchronous action calls. No CLI changes.

Out of scope here (later sessions): collectors still run **sequentially** inside the
gather (S8d.3), slow data is still re-collected each tick (S8d.4), and one-shot action
execution + logs reload still use `block_on` (brief, user-initiated, acceptable).
