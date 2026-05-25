# Phase 9 - v0.9 Policies & expected state

> First session deliverable. Freezes scope and decisions for v0.9 before coding.
> See [`../ROADMAP.md`](../ROADMAP.md), [`../METHODOLOGY.md`](../METHODOLOGY.md),
> and `Product.md` sections 4.15, 7.5 and v0.9.
> Built on `release/v0.9` (from `main` after v0.8.4); tag `v0.9` when the DoD is met.

## Goal

Make SysTUI validate hosts against an explicit expected state, not only observe the
current state. v0.9 introduces policies for the configuration that should or should
not exist on a host, evaluates the already-collected inventory against those policies,
and reports drift as first-class findings in the TUI, CLI reports and fleet workflows.

The phase keeps the existing safety model intact: policy checks are read-only, run over
the shared `Transport`, degrade to partial data when collectors lack permissions, and
do not introduce automatic remediation. By the end of the phase, a web host can say
"port 6379 is open and forbidden by policy" instead of only "port 6379 is open".

## In scope

- **Policy schema** (`systui-core` + config loading): versioned policy definitions in
  the existing TOML config, with host assignment via explicit `policy = "name"` and a
  deterministic tag-based fallback for fleet use.
- **Expected and forbidden network state**: expected open ports, forbidden ports and
  listener exposure checks using the existing network/exposure collectors.
- **Expected and forbidden services**: required systemd services, forbidden services,
  and service state checks using the existing service inventory.
- **Threshold policy checks**: host-specific warning/critical thresholds for disk,
  memory, load and other already-collected health signals where the current global
  heuristics are too generic.
- **Identity and access checks**: expected sudo-capable users and forbidden users where
  the existing collectors/security checks can provide reliable evidence.
- **Certificate expectations**: expected certificate hosts/names and expiry thresholds
  using the existing certificate model, without adding secret storage.
- **Container expectations**: expected and forbidden containers/images where Docker is
  available, based on the existing Docker collectors.
- **Policy evaluation engine** (`systui-security` or a small dedicated module if the
  code shape warrants it): pure evaluation from collected facts plus policy into
  `Finding`s with stable `policy.*` identifiers and evidence.
- **Drift reporting**: render policy drift in the Security view, per-host reports and
  fleet reports/search/comparison where relevant.
- **Exceptions and finding states**: persistent finding lifecycle state
  (`open`, `accepted`, `ignored`, `fixed`, `false-positive`) with scoped exceptions
  that can suppress or downgrade known policy drift without deleting the underlying
  evidence.

## Out of scope (deferred)

- **Automatic remediation / apply fix**: still deferred to v1.1+; v0.9 may show
  recommendations, but it does not apply policy fixes.
- **Continuous monitoring or alerting**: policy checks happen during normal refresh,
  report and fleet gather flows only.
- **Secret management**: no policy field stores passwords, tokens or private keys.
- **A full policy language**: no scripting, expressions or arbitrary command checks.
  v0.9 uses a typed schema that maps to data SysTUI already collects.
- **Compliance frameworks**: no CIS/STIG profile pack in this phase. The schema should
  make future profile packs possible, but v0.9 ships the engine and user-authored
  policies.
- **Large visual redesign**: policy drift appears in the existing Security/report/fleet
  surfaces. New UI is limited to the controls needed for finding state and exceptions.

## Key decisions

- **Typed policy model over string rules.** Policies are structured Rust/TOML data and
  evaluated by typed code. This preserves the no-free-form-command rule and keeps
  evaluation testable.
- **Policies evaluate collected facts only.** The engine does not run host commands
  directly. Collectors gather facts through `Transport`; the policy layer evaluates
  snapshots. This keeps local and SSH behavior identical.
- **Stable finding IDs are mandatory.** Policy drift and finding-state persistence need
  deterministic IDs derived from policy name, host scope and subject
  (`policy.port.forbidden:prod-web:6379`, for example).
- **Exceptions do not erase evidence.** Accepted or ignored drift is still visible in
  reports when requested and remains auditable; state changes are persisted locally and
  should include enough metadata to explain who/when/why once the audit model supports
  it.
- **Host policy resolution is deterministic.** Explicit host policy wins. If a policy
  targets tags, matching order must be stable and documented so fleet runs are
  reproducible.
- **Partial data produces partial policy results.** If a collector is unavailable or
  permission-limited, the evaluator emits an unknown/partial note instead of claiming
  compliance.
- **Reports and fleet are first-class.** Policy checks are not TUI-only; JSON exposes
  structured drift and states, while Markdown/HTML summarize the human-facing result.

## Sessions

- **S9.1 - Context** *(this file)*.
- **S9.2 - Policy schema**: add config policy definitions and host policy resolution;
  cover expected/forbidden ports and services, thresholds, sudo users, expected
  certificates and containers with serialization/unit tests.
  **Done.** Expanded the existing `[policies.<name>]` config model into a versioned,
  typed expected-state schema: explicit/forbidden ports and services, disk/RAM/load
  thresholds, expected sudo users, forbidden users, expected certificate targets,
  expected containers, forbidden containers and forbidden images. Hosts can still set
  `policy = "name"` explicitly; when they do not, `match_tags = [...]` provides a
  deterministic fallback (most matching tags wins, then policy name). `Config`,
  `ResolvedHost` and `FleetHost` now expose the resolved policy name, and
  `resolve_policy_for_host` reports explicit missing policies separately so the later
  evaluator can turn invalid config into isolated per-host drift instead of failing a
  whole fleet run. Serialization and resolution behavior are covered by unit tests.
- **S9.3 - Policy evaluation engine + drift findings**: evaluate gathered host facts
  against policies, produce stable `policy.*` findings, wire results into Security,
  dashboard health, reports and fleet output.
- **S9.4 - Exceptions, finding states and polish**: persist finding states/exceptions,
  add TUI/report affordances for accepted/ignored/fixed/false-positive findings,
  refresh docs/tests/help text, run final gates, merge `--no-ff` into `main` and tag
  `v0.9`.

## Definition of Done

- Hosts can be assigned an expected-state policy from config, locally or through fleet
  inventory, with deterministic resolution.
- SysTUI detects and reports drift for ports, services, thresholds, sudo users,
  certificates and Docker/container expectations where the underlying data is
  available.
- Policy drift appears as stable `Finding`s with severity, evidence and
  recommendations in the TUI and in JSON/Markdown/HTML reports.
- Fleet mode includes policy status in overview/report flows and preserves per-host
  isolation when one host has invalid policy data or partial collector results.
- Finding lifecycle states and policy exceptions are persisted, reflected in the TUI
  and reports, and never require deleting the original finding evidence.
- No policy check bypasses `Transport`, no free-form shell commands are introduced,
  and missing permissions degrade gracefully.
- Parser/evaluator/config behavior is covered by focused unit tests and fixtures where
  command output is involved.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`
  and `cargo test --workspace` pass before release tagging.

## Risks & open questions

- **Policy schema growth:** too many fields can make v0.9 hard to finish. Prefer a
  minimal typed schema that covers the roadmap list and can be extended later without
  breaking existing configs.
- **Fact availability:** sudo users, certificates and containers may be unavailable on
  some hosts. Unknown must be distinct from compliant.
- **Finding state identity:** stable IDs must survive harmless wording/layout changes;
  tests should pin IDs for representative policy drift.
- **Severity mapping:** forbidden public exposure is usually higher risk than a missing
  optional service. The evaluator needs clear defaults and policy-level overrides only
  if the schema stays simple.
- **Fleet policy assignment:** tag-based policy fallback is useful, but ambiguous
  matches can surprise users. The resolution algorithm must be explicit and tested.
- **Report noise:** accepted/ignored findings should not hide risk silently. Reports
  need a concise summary plus enough detail to audit exceptions.
