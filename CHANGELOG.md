# Changelog

All notable changes to SysTUI. The format follows [Keep a Changelog](https://keepachangelog.com),
and the project adheres to [Semantic Versioning](https://semver.org). Each version maps to
a development phase (see [`docs/ROADMAP.md`](docs/ROADMAP.md)).

## [1.0.0] - 2026-05-25

First stable release. No new modules over v0.9 — this version raises the product floor:
tests, hardening, packaging and documentation.

### Added
- Per-distro integration test suite (Debian, Ubuntu, Arch, Fedora, AlmaLinux, Alpine)
  plus a systemd + local/SSH parity job, running the real collectors in CI.
- Property-based fuzzing harness for the command-output parsers (collectors and
  security), asserting parsing never panics on adversarial input.
- Large-journal correctness test and a criterion throughput benchmark for the log parser.
- Release pipeline: static `x86_64`/`aarch64` musl binaries, `.deb`/`.rpm` (nfpm),
  AUR `PKGBUILD`, `install.sh`, SHA256 checksums, keyless Sigstore (cosign) signatures
  and an SPDX SBOM.
- MIT/Apache-2.0 license files, a `systui.1` man page, README, install docs, a written
  security review and a testing/fixture-convention guide.

### Changed
- Workspace version bumped to `1.0.0`.

## [0.9] - 2026-05-25
**Policies & expected state.** Expected/forbidden ports, services, thresholds, sudo
users, certificates and containers; a policy evaluation engine emitting stable
`policy.*` drift findings; finding lifecycle states (accepted/ignored/fixed/false-positive)
and exceptions, persisted and reflected in the TUI and reports.

## [0.8.4] - 2026-05-25
**Prototype data parity.** Services filters, on-demand connectivity tests, a firewall
collector, Docker Compose + image hygiene + prune, packages/updates tile, anacron and
cron run-now, and persistence-backed trend/session-notes/saved-search panels.

## [0.8.3] - 2026-05-25
**Optimization.** Background refresh off the UI thread, concurrent collectors, tiered
refresh/caching, command batching with per-collector timeouts, and release-profile tuning.

## [0.8.2] - 2026-05-24
**UI redesign.** Truecolor single-source theme, three-row chrome, badged tabs and
multi-panel screens matching the approved prototype — real data only, no mocks.

## [0.8.1] - 2026-05-24
**In-TUI management & UX polish.** Add/edit/delete inventory hosts (persisted to
`config.toml`) and engine-mediated user-crontab management, plus a layout polish.

## [0.8] - 2026-05-24
**Fleet.** Host inventory with tags/groups/favorites, concurrent health checks, a global
overview, global search, host comparison and fleet reports (inspection only).

## [0.7] - 2026-05-24
**Databases.** Detection and exposure of PostgreSQL/Redis/MySQL-MariaDB/MongoDB, with
operational panels and credential handling that never stores plaintext secrets.

## [0.6] - 2026-05-24
**Reports.** JSON, Markdown and HTML reports (health, security, ports, docker, services,
inventory) and a `report` CLI subcommand.

## [0.5] - 2026-05-24
**Remote SSH.** `SshTransport` over the system ssh client, host profiles, remote
read-only with permission detection, and full local/remote module parity.

## [0.4] - 2026-05-24
**Docker & crons.** Container collectors, ops and risk checks; cron sources, systemd
timers, expression validation and cron checks.

## [0.3] - 2026-05-24
**Network & security.** Network collectors, port→process→service correlation, the
exposure map, connectivity tools, evidence-based security findings and TLS certificates.

## [0.2] - 2026-05-24
**Local operation.** systemd and process modules, log filters, and the action safety
engine with read-only mode and a JSON audit trail.

## [0.1] - 2026-05-24
**Functional demo.** System collectors, dashboard panels, top processes, failed units,
basic journald logs, threshold checks and a minimal Markdown report.

[1.0.0]: https://github.com/njuante/Systools/releases/tag/v1.0
[0.9]: https://github.com/njuante/Systools/releases/tag/v0.9
[0.8.4]: https://github.com/njuante/Systools/releases/tag/v0.8.4
[0.8.3]: https://github.com/njuante/Systools/releases/tag/v0.8.3
[0.8.2]: https://github.com/njuante/Systools/releases/tag/v0.8.2
[0.8.1]: https://github.com/njuante/Systools/releases/tag/v0.8.1
[0.8]: https://github.com/njuante/Systools/releases/tag/v0.8
[0.7]: https://github.com/njuante/Systools/releases/tag/v0.7
[0.6]: https://github.com/njuante/Systools/releases/tag/v0.6
[0.5]: https://github.com/njuante/Systools/releases/tag/v0.5
[0.4]: https://github.com/njuante/Systools/releases/tag/v0.4
[0.3]: https://github.com/njuante/Systools/releases/tag/v0.3
[0.2]: https://github.com/njuante/Systools/releases/tag/v0.2
[0.1]: https://github.com/njuante/Systools/releases/tag/v0.1
