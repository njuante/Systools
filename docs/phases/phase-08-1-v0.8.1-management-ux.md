# Phase 8.5 — v0.8.1 In-TUI management & UX polish

> First session deliverable. Freezes scope and decisions for v0.8.1 before coding.
> An **intermediate** phase between v0.8 (Fleet) and v0.9 (Policies). See
> [`../ROADMAP.md`](../ROADMAP.md) and [`../METHODOLOGY.md`](../METHODOLOGY.md).
> Built on `release/v0.8.1` (from `main` after v0.8); tag `v0.8.1` when the DoD is met.

## Goal

Close two gaps that keep SysTUI read-only where it should be operable, and give the
TUI a visual pass. Today the inventory and crons can only be **viewed**: SSH hosts
are hand-edited in `config.toml`, and the Crons tab only lists jobs. v0.8.1 makes
both **manageable from inside the TUI**, and polishes the interface so it looks and
reads well.

1. **Manage the SSH inventory from the TUI**: add, edit and delete hosts (`host`,
   `user`, `port`, `tags`, `read_only`), persisted to `config.toml`.
2. **Manage the user crontab from the TUI**: add, edit, delete and enable/disable
   cron entries, through the existing **action engine** (preview → confirm → backup
   → execute → verify → audit), with schedule validation.
3. **TUI layout polish**: a cleaner header/status bar, consistent borders and
   spacing, severity badges, more readable tables, and clearer loading / empty /
   error states.

This is mutation in two new places, so the v0.2 safety model applies: no free-form
command strings (`CommandSpec`), confirmations for destructive edits, backups before
writing, and an audit record for every change.

## In scope

- **Config persistence** (`systui-storage`): write the inventory back to
  `config.toml` — add/update/remove `[hosts.<id>]` entries — **preserving the rest
  of the file** (other tables, comments, formatting). Plus `systui-core` helpers to
  upsert/remove a host in the in-memory `Config`.
- **Inventory management UI**: from the **fleet TUI** (which already lists the
  inventory), `a` add a host (a small multi-field form), `e` edit the selected host,
  `d` delete it (confirmed). Changes persist immediately and the overview re-gathers.
- **A reusable form/input component** for the TUI (multi-field, validated), since
  both host and cron editing need one. Generalises the current single-field action
  modal.
- **Cron actions** (`systui-actions` + `systui-collectors`): a `CronAction` that
  adds, edits, deletes and toggles (comment/uncomment) entries in the **user
  crontab**, via `crontab -l` / `crontab -` (stdin) wrapped in `CommandSpec`. Cron
  expression **validation** reuses `parse_schedule`/`CronSchedule`. The previous
  crontab is **backed up** before writing.
- **Cron management UI**: in the **Crons tab**, `a` add (form: schedule + command),
  `e` edit, `d` delete, toggle enable/disable on the selected user-crontab entry,
  each routed through the action engine (preview/confirm/audit), then refresh.
- **Layout polish**: refined title + status bar, consistent block borders/padding,
  severity **badges**, aligned/readable tables, and distinct loading/empty/error
  states — applied across existing tabs and the new forms. Runs over local and SSH
  identically (host crontab is reached through the `Transport`).

## Out of scope (deferred)

- **System-wide cron sources**: `/etc/crontab`, `/etc/cron.d`, `cron.*` and
  **systemd timers** stay **read-only** this phase (editing them is higher-risk and
  often root-owned). Only the **user crontab** is writable.
- **Editing arbitrary config sections** from the TUI (thresholds, policies, ui) —
  only the `[hosts.*]` inventory is editable here.
- **Command palette (Ctrl-k)**, **theme/colour switching** and a **reworked help
  overlay** — considered but *not selected* for this phase; the redesign is scoped to
  **layout polish**. (Palette + themes remain good candidates for a later UX phase.)
- **Policies / expected-state** — that is phase 9 (v0.9), unchanged.
- **Fleet-level mutation** (mass host/cron edits) — fleet stays inspection + the new
  per-host management; no bulk destructive operations.

## Key decisions

- **Surgical config writes, not a full rewrite.** Persisting the inventory must not
  clobber the user's hand-maintained `config.toml` (comments, ordering, other
  tables). Use **`toml_edit`** to edit only the `[hosts.<id>]` tables in place. This
  adds one well-scoped dependency; the alternative (serialize the whole `Config`
  back) would erase comments and is rejected.
- **Cron edits go through the action engine — no exceptions.** Adding/editing a cron
  is a mutation: it must produce a `CommandSpec` (never a shell string), show a
  preview, require confirmation for destructive changes, **back up the prior
  crontab**, and write an audit record — exactly the v0.2 contract. Writing uses
  `crontab -` with the new content piped via `CommandSpec::stdin` (added in v0.3), so
  there is no shell interpolation of user input.
- **User crontab only.** `crontab -l`/`crontab -` operate on the connected user and
  need no privilege, keeping the feature safe-by-default and identical local/remote.
  System cron + timers remain read-only (documented above).
- **Reuse, don't fork.** The cron form validates with the existing
  `parse_schedule`; host management lives in the existing fleet TUI; the form
  component generalises the existing action-modal input rather than adding a parallel
  input system.
- **Read-only mode and per-host `read_only` still bind.** In read-only mode (or a
  `read_only` host) the management actions are disabled/blocked with a clear message,
  just like other mutations.
- **Polish is non-functional.** The layout pass must not change behaviour or break
  the headless/`TestBackend` render tests; it refines styling and the shared
  loading/empty/error states only.

## Sessions

- **S8b.1 — Context** *(this file)* + ROADMAP insert for v0.8.1.
- **S8b.2 — Config persistence + inventory model**: `systui-storage` write via
  `toml_edit` (add/update/remove `[hosts.<id>]`, preserving the file);
  `systui-core` host upsert/remove helpers; tests (round-trip, comment preservation).
  **Done.** `systui-core::Config` gains `upsert_host`/`remove_host` (in-memory).
  `systui-storage` adds `save_host`/`save_host_to` and `remove_host`/`remove_host_from`
  using **`toml_edit`** for surgical `[hosts.<id>]` edits: only SysTUI-managed keys are
  written (optional/false fields removed so entries stay minimal), the rest of the
  file — other tables, comments, ordering — is preserved, and writes are **atomic**
  (temp file + rename). Missing file/dir are created. Tested: comment/other-table
  preservation, create-on-missing, clearing optional fields on update, remove
  existence reporting, and dropping the empty `[hosts]` table.
- **S8b.3 — Host management in the fleet TUI**: a reusable multi-field form modal;
  `a`/`e`/`d` to add/edit/delete inventory hosts; persist + re-gather; read-only
  guard.
  **Done.** New `systui-ui::form` — a reusable modal of labelled text/bool fields
  (focus nav, inline edit, error line, `render_form`), unit-tested. The fleet TUI now
  manages the inventory: `a` add (form with id/host/user/port/tags/read_only/favorite,
  validated: required id without spaces, no duplicate, valid port, comma-split tags),
  `e` edit the selected host (id fixed, **policy preserved**), `d` delete (y/n
  confirm). Edits persist via S8b.2 (`save_host_to`/`remove_host_from`) and mirror
  into the in-memory `Config`, then the view **re-gathers** through a caller-supplied
  closure (no screen exit). In **read-only mode** the management keys show a notice
  and do nothing. `run_fleet` now takes `&mut Config` + `config_path` + `read_only` +
  the gather closure; the CLI threads the resolved config path and gathers lazily
  (headless modes still gather eagerly). `build_host` validation and the form
  helpers are unit-tested; the render stays `TestBackend`-tested.
- **S8b.4 — Cron actions**: `CronAction` (add/edit/delete/toggle on the user crontab)
  with schedule validation, backup and audit, built on `CommandSpec::stdin`;
  fixture-tested crontab read/modify/write.
  **Done.** `systui-actions::cron::CronAction` implements the `Action` contract for
  the **user crontab** (`CronOp` Add/Edit/Delete/Enable/Disable): `preview` validates
  the schedule via `parse_schedule` (invalid → rejected before any write); `execute`
  reads `crontab -l`, computes the new crontab with **pure, tested transforms**
  (append / remove / replace / comment-toggle, matching entries by parsed
  schedule+command, reusing `parse_crontab`), **backs up** the prior crontab to
  `/tmp/systui-crontab.bak` via `tee`, then installs via `crontab -` with the content
  piped through `CommandSpec::stdin` (no shell interpolation). Risk: Delete High,
  Enable Low, others Medium; no privilege (user-owned crontab). It flows through the
  engine like every mutation (mode gate, confirmation, audit). System cron and timers
  remain read-only. Transforms + execute paths (read/backup/install, missing-entry
  failure, invalid-schedule rejection) are unit-tested with `MockTransport`.
- **S8b.5 — Cron management in the Crons tab**: wire the form + `CronAction` through
  the engine (preview/confirm/audit) into the Crons tab; refresh after.
- **S8b.6 — TUI layout polish + close**: header/status bar, borders/spacing,
  severity badges, readable tables, loading/empty/error states across tabs; final
  gates; merge `--no-ff` into `main` + tag `v0.8.1`.

## Definition of Done

- From the TUI, an operator can **add, edit and delete SSH inventory hosts**, and the
  change is saved to `config.toml` without destroying the rest of the file.
- From the TUI, an operator can **add, edit, delete and enable/disable user-crontab
  entries**, each going through the action engine (preview → confirm → backup →
  execute → verify → audit) with cron-expression validation; system cron and timers
  remain read-only.
- All mutations respect read-only mode and per-host `read_only`.
- The TUI has a **visibly cleaner layout**: header/status bar, consistent
  borders/spacing, severity badges, readable tables, clear loading/empty/error
  states — with the render tests still passing.
- `cargo fmt --check`, `cargo clippy -D warnings` and `cargo test --workspace` pass;
  crontab parsing/round-trip and config writes are fixture/round-trip tested.

## Risks & open questions

- **Config-file integrity**: a botched write could corrupt `config.toml`. Mitigate
  with `toml_edit` (surgical edits), write-to-temp-then-rename, and a round-trip test;
  never serialise-and-overwrite the whole file.
- **Crontab clobbering**: `crontab -` replaces the entire crontab, so a bad edit can
  wipe jobs. Always read current → modify the target line → write back, **after
  backing up** the prior crontab; validate the schedule before writing.
- **Quoting/injection in cron commands**: user-entered schedules/commands must never
  be shell-interpolated. The whole new crontab is piped via `CommandSpec::stdin`; the
  command text is stored verbatim as a crontab line, not executed by SysTUI.
- **Remote crontab differences**: `crontab` availability/behaviour varies (busybox,
  no MTA). Degrade gracefully and surface a clear error rather than assuming success;
  verify via re-read after write.
- **Form UX scope creep**: a generic form widget can balloon. Keep it minimal —
  labelled text fields, tab/enter navigation, inline validation — enough for host and
  cron editing, no more.
- **Polish vs. regressions**: restyling shared widgets risks breaking other tabs or
  the `TestBackend` tests. Keep styling centralised (theme/helpers), change behaviour
  nowhere, and re-run the render tests.
