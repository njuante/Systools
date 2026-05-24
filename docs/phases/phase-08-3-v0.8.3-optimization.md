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
