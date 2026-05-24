# Phase 7 - v0.7 Databases

> First session (S7.1) deliverable. Freezes scope and decisions for v0.7 before
> coding. See [`../ROADMAP.md`](../ROADMAP.md) and
> [`../METHODOLOGY.md`](../METHODOLOGY.md). Built on `release/v0.7` from `main`
> after `v0.6`; tag `v0.7` when the DoD is met.

## Goal

Add operational visibility for critical database services without turning SysTUI
into a SQL client. An operator should quickly see whether PostgreSQL, Redis,
MySQL/MariaDB or MongoDB is present, running, exposed, healthy enough for basic
operations, and producing recent errors, locally or over SSH.

## In scope

- **Database discovery** for PostgreSQL, Redis, MySQL/MariaDB and MongoDB:
  service/unit state, listening ports, process ownership, version and network
  exposure.
- **Exposure checks** that reuse existing network/service correlation where
  possible: public listeners on default/sensitive ports, Redis without an
  authentication signal, and database ports reachable outside loopback.
- **Operational snapshots** focused on read-only inspection: connections,
  approximate database/keyspace/table sizes, recent error logs, replication
  status and lock/blocked-client indicators where the local tooling permits it.
- **Credential handling** that never stores plaintext passwords: local sockets,
  existing `.pgpass`, existing `mysql_config_editor`, environment variables, and
  temporary in-memory prompts later. Missing credentials must produce partial
  data, not a failed refresh.
- **TUI database tab** with a concise list and detail pane for detected engines,
  exposure, operational signals and findings.
- **Report integration** so v0.6 exports include a database section when database
  data is available.
- **Remote parity** through `Transport`: every collector command must work over
  `LocalTransport`, `SshTransport` and `MockTransport`.

## Out of scope (deferred)

- A general SQL shell, query editor or schema browser.
- Storing database credentials, connection strings or secrets in SysTUI config.
- Mutating database operations such as kill query, failover, vacuum, repair,
  restart or configuration edits.
- Secret-manager integrations; leave a future extension point but do not add a
  dependency this phase.
- Deep engine-specific tuning advice beyond clear operational warnings.
- Backup orchestration; v0.7 may detect likely backup evidence only.

## Key decisions

- **Discovery first, credentials second.** S7.2 must provide useful data from
  services, processes and ports without credentials. Authenticated probes only
  enrich the snapshot.
- **No plaintext secrets.** Collectors may consume existing host mechanisms
  (`.pgpass`, socket auth, `mysql_config_editor`, env vars) but SysTUI does not
  persist database passwords or echo them into reports.
- **Use native CLIs when available.** `psql`, `redis-cli`, `mysql`/`mariadb` and
  `mongosh`/`mongo` are optional enrichers. Absence of a CLI records partial data
  instead of failing the module.
- **Reuse existing evidence models.** Database exposure and health issues should
  become `Finding`s so they appear in the dashboard, reports and future policy
  work without a parallel warning model.
- **Transport boundary stays intact.** Database collectors only execute
  `CommandSpec`s through `Transport`; no UI command execution and no free-form
  shell strings.
- **Parser fixtures are mandatory.** Any parser for CLI output or config snippets
  gets fixture tests before it is considered complete.

## Sessions

- **S7.1 - Context** *(this file)*.
- **S7.2 - Detection & exposure**: add database engine models and collectors for
  PostgreSQL, Redis, MySQL/MariaDB and MongoDB service/port/version/state/exposure;
  add fixture tests and surface findings for risky exposure.
- **S7.3 - Operational panels**: add read-only operational probes for
  connections, sizes, recent errors and replication/lock indicators; render a
  Databases TUI tab with list/detail state.
- **S7.4 - DB checks + credentials handling**: add safe credential-source handling
  and engine-specific health checks, integrate database output into reports,
  polish, merge `--no-ff` into `main` and tag `v0.7`.

## Definition of Done

- SysTUI detects PostgreSQL, Redis, MySQL/MariaDB and MongoDB locally and over
  SSH when present.
- The database view shows service state, port, version, exposure and core
  operational signals, degrading gracefully when permissions or CLIs are missing.
- Database findings cover at least public exposure, Redis authentication risk,
  excessive connections/blocked clients where known, broken replication where
  known, recent errors and data-disk pressure where detectable.
- No database password is stored in plaintext or written to reports, logs or audit
  events.
- Reports include detected database state and findings.
- Parser fixtures cover command outputs used by database collectors.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D
  warnings` and `cargo test --workspace` pass.

## Risks & open questions

- **Credential ambiguity**: each engine has several auth paths. Start with
  no-secret discovery and clearly mark authenticated details as unavailable when
  credentials are absent.
- **CLI variability**: `psql`, `redis-cli`, `mysql`, `mariadb`, `mongosh` and
  `mongo` output differs by version and distro. Keep parsers small, prefer stable
  machine-readable flags where available, and fixture every format.
- **Remote quoting**: database probes may need stdin or env vars. Keep using
  `CommandSpec` and the existing SSH transport boundary rather than introducing
  shell-specific command strings.
- **False exposure positives**: a public listener can be protected by firewall or
  bind rules. Findings should describe observed evidence instead of claiming
  confirmed internet reachability.
- **Report redaction**: database names, users and paths are useful but can be
  sensitive. Avoid including connection strings or credential-bearing environment
  values.
