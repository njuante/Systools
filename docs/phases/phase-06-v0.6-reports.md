# Phase 6 — v0.6 Reports

> First session (S6.1) deliverable. Freezes scope and decisions for v0.6 before
> coding. See [`../ROADMAP.md`](../ROADMAP.md) and [`../METHODOLOGY.md`](../METHODOLOGY.md).
> Built on `release/v0.6` (from `main` after v0.5); tag `v0.6` when the DoD is met.

## Goal

Make SysTUI useful for documentation, audits and handover: after reviewing a
server an operator can export **a single, useful report** of its state — health,
security findings with evidence and recommendations, exposed ports, problematic
containers, failed services, crons and host inventory — in **JSON, Markdown or
HTML** (`Product.md` §8 v0.6, §11 "Fase 11", §7.6). One command, local or over
SSH:

```sh
systui report --host prod-01 --format markdown
systui report --host prod-01 --security --format html
```

v0.1 already renders a basic health report as Markdown (`systui-report::to_markdown`
over `HostReport`). v0.6 turns that seed into a real reporting feature: a richer
report model, three formats, the full set of sections, the `report` CLI wired for
local and remote hosts, and session notes.

## In scope

- **A report data model** (`Report`) that bundles everything a review produces:
  metadata (host label, generated-at, execution mode, detected capabilities), the
  existing `HostReport` (system / health / processes / failed units / logs), the
  network snapshot + exposure map, the merged security findings (security + docker
  + cron, worst-first), the docker containers/inspects, the cron entries/timers, the
  host inventory entry, and any **session notes**. Serializable for JSON.
- **Headless gathering**: a `gather_report(transport, config, …)` that runs the
  collectors and the `systui-security` scans over any `Transport`, so the `report`
  CLI works without the TUI — local or over the v0.5 `SshTransport`.
- **Three renderers**, each a pure function of `Report`:
  - **JSON** — the full structured model (`serde_json`, pretty-printed), for tooling.
  - **Markdown** — extend the current renderer into sections: executive summary,
    overall state/health, security findings (with evidence + recommendation), open
    /exposed ports, problematic containers, failed services, crons and host
    inventory.
  - **HTML** — a single **self-contained** file with inline CSS (no external assets,
    no template engine), readable and printable.
- **Report scope flags**: a **full** report by default and `--security` for a
  security-focused report; `--output <file>` writes to a file (otherwise stdout).
- **Session notes** (`Product.md` §7.10): jot `[NOTE]` lines during a TUI review and
  have them appear in an exported report; the CLI can also accept notes.
- **`report` CLI subcommand**: `systui report [--host <id|user@host>] --format
  <json|markdown|html> [--security] [--output <file>] [--note <text>]`.

## Out of scope (deferred)

- **PDF** output — explicitly a later phase (`Product.md` §11).
- **Fleet / global reports** across many hosts — phase 8 (`Product.md` §11 "Fleet
  report").
- **Scheduled / automated** report generation, report **diffing** and historical
  comparison.
- **Command palette** ("security report" via Ctrl-k) — a separate UX item.
- **Policy/expected-state** sections (drift) — phase 9; the report describes
  observed state, not compliance yet.
- **Per-section à-la-carte flags** beyond `--security` (e.g. `--ports`, `--docker`)
  — keep the flag surface small this phase.

## Key decisions

- **Model + gather + render are separated.** `Report` is a plain serializable data
  model; `gather_report` does all the I/O (collectors + security scans over a
  `Transport`); the three renderers are **pure functions** of `Report`. This keeps
  the renderers golden-/fixture-testable and deterministic.
- **Determinism**: the caller injects `generated_at` (already the pattern in
  `to_markdown`), and findings are already sorted worst-first, so report output is
  reproducible and diff-friendly.
- **HTML is hand-rendered, self-contained, offline.** Inline CSS, no template engine,
  no external assets or JS — one portable file, consistent with the single-binary
  ethos. All host-derived text (hostnames, evidence, log lines, commands) is
  **HTML-escaped**: SysTUI renders strings collected from untrusted remote hosts, so
  escaping is a security requirement, not a nicety.
- **JSON via `serde_json`** (already a workspace dependency); `Report` derives
  `Serialize`. JSON is the source of truth that MD/HTML summarise.
- **Reporting is read-only.** Generating a report is pure inspection — no actions —
  so it is always safe, even against a production host in read-only mode.
- **Remote reuses v0.5.** `--host` resolves through `Config::resolve_target` and runs
  over `SshTransport`; the report metadata records the probed `HostCapabilities` so a
  partial report (limited permissions) is self-explanatory.
- **`systui-report` gains a dependency on `systui-security`** (and uses the existing
  collectors) for `gather_report`. The TUI keeps its own collection path in
  `systui-ui::data` for now; the overlap is acknowledged and is a candidate for later
  unification rather than a blocker.
- **Sections cap large data** (top-N processes, recent logs, listeners) the same way
  the dashboard does, so reports stay readable.

## Sessions

- **S6.1 — Context** *(this file)*.
- **S6.2 — Report model + JSON**: define `Report` (+ metadata) and `gather_report`
  over a `Transport`; `to_json` pretty export. Fixture-tested via `MockTransport`.
- **S6.3 — Markdown reports**: expand `to_markdown` into the full section set
  (executive summary, health, security w/ evidence + recommendations, open ports,
  docker, failed services, crons, host inventory).
- **S6.4 — HTML reports**: a single self-contained, styled, printable HTML file with
  escaped content; golden-tested.
- **S6.5 — `report` CLI** (local + remote, `--format`, `--security`, `--output`) +
  **session notes** + polish → **tag v0.6** (final session: merge `--no-ff` into
  `main` + tag).
  **Done.** `systui report [--host <id|user@host>] --format markdown|json|html
  [--security] [-o FILE] [--note TEXT]…` gathers a `Report` locally or over the v0.5
  `SshTransport` (honouring a per-host `read_only` profile) and renders it. A
  `ReportScope` drives `--security`, dropping operational sections (health, services,
  crons, inventory) from the human formats while JSON stays the full model. Session
  notes arrive via repeatable `--note` and appear in every format; `--output` writes
  to a file, otherwise stdout. Verified end-to-end against the local host in all
  three formats.

## Definition of Done

- `systui report [--host <target>] --format markdown|json|html` produces a useful
  report for a server, local or over SSH.
- The report includes an executive summary / overall state, health, security
  findings **with evidence and recommendations**, open/exposed ports, problematic
  containers, failed services, crons and host inventory metadata.
- JSON is the full structured model; Markdown and HTML are human-readable; HTML is a
  single self-contained, escaped file.
- `--security` scopes to a security-focused report; `--output <file>` writes to a
  file (else stdout).
- Session notes captured during a review appear in the report.
- Renderers are pure and golden-/substring-tested; `cargo fmt --check`,
  `cargo clippy -D warnings` and `cargo test --workspace` pass.

## Risks & open questions

- **Collection-path duplication**: `gather_report` overlaps with `systui-ui::data`'s
  refresh. Risk of divergence (a module added to one, missed in the other). Mitigate
  by keeping `gather_report` the single headless path and noting the future
  unification; do not fork collector logic.
- **HTML injection / breakage**: remote-collected strings must be escaped before
  embedding; an unescaped log line or hostname could break or poison the HTML. Treat
  escaping as mandatory and test it.
- **Report size**: logs, processes and listeners can be large; cap sections (top-N)
  to keep MD/HTML readable, while JSON may carry more.
- **Remote latency**: a full report fires many sequential `ssh` commands; acceptable
  for a one-shot export, but note OpenSSH `ControlMaster` reuse as a future speed-up.
- **Session notes plumbing**: notes originate in a TUI review but a CLI report has no
  session — define the seam clearly (TUI export includes its notes buffer; CLI takes
  `--note`) so the feature stays small and unambiguous.
- **Partial data over SSH**: a limited remote user yields gaps; the report must say
  so (via the capability metadata) rather than implying a clean host.
