# Phase 2 — v0.2 Local operation

> First session (S2.1) deliverable. Freezes scope and decisions for v0.2 before
> coding. See [`../ROADMAP.md`](../ROADMAP.md) and [`../METHODOLOGY.md`](../METHODOLOGY.md).
> Built on `release/v0.2` (from `main` after v0.1); tag `v0.2` when the DoD is met.

## Goal

Turn SysTUI from a read-only dashboard into a tool you can *operate* with — safely.
You should be able to find a failed service, read its logs, and restart it from the
TUI, with a preview, an explicit confirmation, and an audit trail — never by
accident. This is where the **action engine** and the **security model** become
real (`Product.md` §3, §8 v0.2, §10).

Everything still goes through the `Transport`/`CommandSpec` contracts; the UI only
*requests* actions, the engine decides whether and how to run them.

## In scope

- **systemd module (full)**: list units, filter (active/failed/enabled/disabled),
  unit detail (status, pid, enabled, unit file path, dependencies), and the
  actions start/stop/restart/reload/enable/disable/mask/unmask.
- **Processes module (full)**: full list (already top-N), process tree, detail
  (cmdline/cwd/open files when permitted), and signals (SIGTERM, then SIGKILL,
  SIGHUP) with guardrails.
- **Log filters**: filter by level/priority, by unit/service, by time window, and
  by regex; incremental in-view search.
- **Action engine** (`systui-actions`): the single path for every mutation —
  permission check → read-only check → risk classification → preview → confirmation
  → backup (if applicable) → execute → verify → audit (`Product.md` §10).
- **Read-only mode enforcement**: `--read-only` (and per-host `read_only`) blocks
  every mutating action at the engine, with a clear message.
- **Audit log** (`systui-storage`): append a JSON record per attempted/executed
  action (timestamp, host, user, module, action, target, status, duration).
- **Contextual confirmations** for dangerous actions (type the action, e.g.
  `restart nginx`), not a generic y/N.

## Out of scope (deferred)

- Network/exposure, security findings, certificates → phase 3.
- Docker, crons → phase 4.
- SSH/remote → phase 5 (actions are designed transport-agnostic, but only local is
  exercised here).
- Cron/unit editing with diffs/rollback beyond what services need → later phases.
- Reports beyond v0.1 Markdown → phase 6.

## Key decisions

- **UI requests, engine decides.** The UI emits an `ActionRequest`; `systui-actions`
  runs the pipeline and returns an outcome. The UI never calls `systemctl`/`kill`.
- **`Action` trait already exists in core.** v0.2 implements concrete actions
  (restart service, signal process, …) and the engine that drives them.
- **Confirmation lives in the UI, authorization in the engine.** The engine returns
  a preview + required confirmation phrase; the UI collects it; the engine verifies
  it before executing. Read-only/permission checks happen engine-side regardless.
- **Guardrails (hard blocks):** never signal PID 1, the SysTUI process itself, or
  core units (`systemd`, `sshd`); killing/restarting other critical targets requires
  the strong (typed) confirmation.
- **Privilege escalation:** the deferred phase-0 question. For v0.2/local, run
  actions as the current user; if a privileged action fails with EPERM, surface a
  clear permission-denied result. A configurable `sudo` escalation is a later
  enhancement (it must not break the TUI with an interactive password prompt).
- **Audit format:** JSON lines at `~/.local/share/systui/audit.log` (one object per
  line), matching `Product.md` §3. Writing is best-effort and must never crash the
  app, but a failed write is itself surfaced.
- **Verification:** after an action, re-query the target (e.g. unit active-state) to
  confirm the intended effect; record the verified state in the outcome/audit.
- **Parsers tested with fixtures** (systemctl status/show, process tree) as always.

## Sessions

- **S2.1 — Context** *(this file)*.
- **S2.2 — systemd module** *(done)*: `ServiceCollector` (full list), `unit_detail`
  (`systemctl show`) and a unified `ServiceUnit`; service actions
  (`ServiceAction`/`ServiceOp`, all eight ops) in `systui-actions` implementing the
  `Action` contract with preview + execute + verify. Fixture/Mock tests. (UI list +
  action invocation are wired once the engine lands in S2.5.)
- **S2.3 — Processes module** *(done)*: `Process` gains PPID; `build_process_tree`
  flattens a depth-annotated tree; `process_detail` reads `/proc/<pid>`
  (status + cmdline). `SignalAction`/`Signal` (TERM/KILL/HUP) in `systui-actions`
  with hard guardrails (PID 1 and the SysTUI process are never signaled) and
  post-signal verification. Fixture/Mock tested. (UI tree/detail/invocation land
  with the engine in S2.5+.)
- **S2.4 — Log filters** *(done)*: `LogQuery` parameterizes `LogsCollector`
  (priority/unit/time/lines, server-side). Logs tab: `l` cycles level, `t` cycles
  time window (re-collect), `/` enters incremental case-insensitive regex search
  (client-side) with a filter bar. Unit filter is collector-ready; its UI picker is
  deferred to a later session.
- **S2.5 — Action engine** *(done)*: `ActionEngine` drives the pipeline —
  guardrail → read-only/permission → risk → preview → confirmation → (backup) →
  execute → verify. `plan` returns Rejected / NeedsConfirmation / Ready; `execute`
  re-checks and verifies the typed phrase. Transport-agnostic. Audit persistence
  is wired in S2.6; UI invocation in S2.7.
- **S2.6 — Read-only mode + audit log** *(done)*: read-only is enforced by the
  engine (S2.5). Added the `AuditRecord`/`AuditStatus`/`AuditContext` model (core),
  `Action::target()`, `ActionEngine::run` (executes + builds a record), and
  `systui-storage::AuditLog` (append JSON lines to `~/.local/share/systui/audit.log`).
  UI persistence of records lands with action invocation in S2.7.
- **S2.7 — Contextual confirmations + guardrails** *(done)*: Services/Processes tabs
  gain row selection (↑/↓); `a` proposes the default action (restart / SIGTERM) and
  opens the action modal → preview → typed confirmation (risky) or Enter (low risk)
  → engine `run` → audit record persisted → result. Read-only/guardrail rejections
  show in the modal. (Op menus for other systemd ops/signals are a follow-up.)
  → **tag v0.2**.

**Phase 2 complete — v0.2.** All sessions done; DoD met. Ready to merge to `main`
and tag `v0.2`.

## Definition of Done

- From the TUI: locate a failed unit, open its recent logs, and restart it through
  the action engine with a preview and confirmation; the action is audited.
- In read-only mode, every mutating action is blocked with a clear message and no
  side effects.
- Guardrails prevent signaling protected processes/units without strong confirmation.
- The audit log records each executed action with result and duration.
- Log filters (level/unit/time/regex) and in-view search work on the Logs view.
- All new parsers have fixture tests; `cargo fmt --check`, `cargo clippy -D warnings`
  and `cargo test --workspace` pass.

## Risks & open questions

- **Privilege/sudo UX** is the main open question — decide the exact behavior in
  S2.5 (block vs. attempt-and-report vs. configurable escalation).
- **Action atomicity/verification:** some actions take time to settle; define how
  long to wait and what "verified" means per action.
- **Confirmation phrasing** must be unambiguous and safe to type; keep a single,
  documented format.
- **systemctl output parsing** varies; prefer machine-readable forms (`show
  --property=...`, `list-units --output=json` where available) and capture fixtures.
- **Audit log growth/rotation** — out of scope for v0.2, but note size could grow;
  revisit later.
