# Phase 10 - v1.0 Stabilization & release

> First session deliverable. Freezes scope and decisions for v1.0 before coding.
> See [`../ROADMAP.md`](../ROADMAP.md), [`../METHODOLOGY.md`](../METHODOLOGY.md),
> and `Product.md` sections 6 (Fase 14-15), 8 (v1.0) and 12.
> Built on `release/v1.0` (from `main` after v0.9); tag `v1.0` when the DoD is met.

## Goal

Turn SysTUI from a feature-complete tool into a finished product that a third party
can install and use without the author standing next to them. The functional surface
is already built across v0.1-v0.9 (system, processes, services, logs, network, docker,
crons, security, certificates, databases, packages, reports, profiles, fleet, policies,
the action safety engine, read-only mode and the local audit/state store). v1.0 adds no
new modules. Instead it raises the floor: a test suite that proves the parsers and the
safety model behave across distros, hardening against malformed and adversarial input,
trivial installation on real Linux systems, and documentation good enough to onboard a
stranger.

By the end of the phase SysTUI is stable (no crashes on bad input or missing
permissions), documented (man page, README, examples, changelog), installable in under a
minute on the main distros (static binaries, AUR, `.deb`, `.rpm`, `cargo install`,
`install.sh`) with verifiable artifacts (checksums, signatures, SBOM), and the workspace
version is bumped from the placeholder `0.1.0` to `1.0.0`.

## In scope

- **Test coverage**: raise unit-test coverage on the layers that matter for correctness
  and safety - command-output parsers (golden/fixture files per distro flavor), the
  action engine (permission -> read-only -> risk -> preview -> confirm -> backup ->
  execute -> verify -> audit), the policy evaluator, and finding/severity logic. Every
  parser that reads command output gets fixtures (this is already a methodology rule;
  v1.0 closes any gaps).
- **Integration tests**: containerized runs against the target distros
  (Debian stable, Ubuntu LTS, Arch, Fedora, Rocky/Alma, Alpine partial) exercising the
  read-only collectors over `LocalTransport`; a systemd-capable environment (container
  or VM) to exercise real unit listing/state. SSH parity smoke-tested via a container
  acting as a remote host.
- **Hardening**: parser fuzzing (malformed/truncated/huge command output must never
  panic), large-log benchmarks (the logs view must stay responsive on multi-hundred-MB
  journals), and a security review of the privileged/action paths and any place that
  shells out, confirming the no-free-form-command invariant holds end to end.
- **Packaging & distribution**: static `x86_64` and `aarch64` Linux binaries, an AUR
  package, `.deb` and `.rpm` packages, `cargo install systui`, an `install.sh`, plus
  release artifacts - checksums, signatures and an SBOM. Optional CI/report Docker image
  if it falls out cheaply.
- **Docs & launch**: a man page, a real README (what it is, install, quick start,
  safety model), usage examples, a demo GIF/asciinema, a changelog assembled from the
  version history, and a CI/CD pipeline that builds, tests, packages and publishes
  release artifacts. Workspace version bump to `1.0.0`.

## Out of scope (deferred)

- **Any new functional module or collector.** v1.0 stabilizes the existing feature set;
  new capabilities wait for v1.1+.
- **Automatic remediation / `apply fix`.** Still deferred (v1.1+), consistent with v0.9.
- **Advanced fleet, Kubernetes, plugin SDK, optional agent, continuous alerting,
  secret-manager integrations, advanced PDF export, web dashboard.** All explicitly
  v1.1+ per the roadmap's "Out of scope until v1.1+".
- **The command palette / fuzzy search** carried over from v0.8.2 stays deferred unless
  it becomes trivial during polish; it is not a release blocker.
- **Homebrew/Linuxbrew** packaging is optional/best-effort, not a DoD requirement.

## Key decisions

- **No new features after this point.** v1.0 is a quality gate, not a feature phase. Any
  behavior change is a bug fix or a hardening fix, not an enhancement. This keeps the
  release finishable and the test/doc effort bounded.
- **Parsers are the test priority.** SysTUI's correctness lives in turning command
  output into typed data across distros. Golden files and fuzzing target this layer
  first; a parser that reads text without a fixture test is treated as a release bug.
- **Integration tests run in containers, in CI.** Per-distro behavior is proven by
  running the read-only collectors in distro containers rather than trusting local dev
  environment. systemd-specific behavior uses a systemd-capable container/VM. This makes
  the distro matrix reproducible and CI-enforceable.
- **No panic on hostile input.** After hardening, malformed/truncated/oversized command
  output produces partial data or a graceful error, never a panic or a crash - the same
  "partial data, never crash" contract the architecture already mandates for missing
  permissions, extended to bad data.
- **Release artifacts are verifiable.** Every published binary ships with a checksum and
  a signature, and the release carries an SBOM. Installation instructions point at
  verifiable artifacts, not opaque blobs.
- **Static binaries for portability.** Linux release binaries are statically linked
  (musl where practical) so a single download runs across distros without runtime
  dependency surprises - matching the single-binary principle from `Product.md`.
- **Version bump is part of the release.** The workspace `version` moves from the
  placeholder `0.1.0` to `1.0.0` in this phase; the tag `v1.0` marks the first version
  whose manifest version matches its release name.
- **CI grows, never shrinks the gates.** The existing fmt/clippy/test gate stays; v1.0
  adds the distro matrix, packaging and release publishing on top, and `main` must stay
  green throughout.

## Sessions

- **S10.1 - Context** *(this file)*.
- **S10.2 - Test coverage**: close unit-test and fixture/golden-file gaps on parsers,
  the action engine, the policy evaluator and finding/severity logic; document the
  fixture convention for future distros.
  **Done.** Audited coverage: every production `parse_*` is already exercised by a
  fixture/unit test and most collectors have edge-case tests, so the real gaps were in
  the safety layer and the docs. Added `docs/TESTING.md` documenting the test layers and
  the fixture/golden-file convention (how fixtures are loaded with `include_str!` +
  `MockTransport`, naming, and how to add a new distro by capturing fixtures), linked
  from `METHODOLOGY.md`. Closed three action-engine gaps: the `Failure` audit status
  (execution runs but does not succeed, distinct from `Rejected`), guardrail enforcement
  on the `execute` path (previously only covered on `plan`), and case/whitespace-
  insensitive confirmation matching.
- **S10.3 - Integration tests**: containerized per-distro runs of the read-only
  collectors (Debian/Ubuntu/Arch/Fedora/Rocky-Alma/Alpine), a systemd-capable
  environment for real unit behavior, and an SSH parity smoke test, wired into CI.
  **Done.** Added a feature-gated integration test target
  (`crates/systui-collectors/tests/integration.rs`, behind the new `integration`
  feature so `cargo test --workspace` stays hermetic) that runs the real collectors over
  `LocalTransport`/`SshTransport`: system-snapshot sanity, PID 1 presence, host-report
  assembly, transport smoke, systemd behaviour (graceful when absent, strict under
  `SYSTUI_HAS_SYSTEMD=1`), and local/SSH hostname+kernel parity (opt-in via
  `SYSTUI_SSH_TARGET`). Added `.github/workflows/integration.yml`: a distro-matrix job
  runs the universal tests inside each distro container (Alpine non-blocking), and a
  second job runs the strict systemd + local/SSH parity tests directly on the
  `ubuntu-latest` runner — where systemd is PID 1 — instead of the fragile
  systemd-in-Docker approach. The fast push gate (`ci.yml`) is unchanged.
- **S10.4 - Hardening**: parser fuzzing harness, large-log benchmarks, and a security
  review of the privileged/action/shell-out paths confirming the no-free-form-command
  invariant; fix what they surface.
  **Done.** Added a shared property-based fuzzing harness in `systui-testkit::fuzz`
  (arbitrary text incl. control bytes/newlines + table-shaped adversarial output) and
  wired robustness tests (400 cases each, in the normal gate) into the parsers across
  collectors (system/network/service/logs/packages/firewall/cron) and security
  (certs/sshd/failed-logins/stat+iptables/sudoers) asserting the invariant *parsing
  never panics*. No panics surfaced. Added a correctness-at-scale test (100k journal
  entries) plus a criterion benchmark (`benches/log_parse.rs`) showing linear log-parse
  throughput (~0.9M entries/s, ~220 ms for 200k lines). Wrote `docs/SECURITY-REVIEW.md`:
  audited the execution paths and confirmed no `std::process::Command` outside the
  transport, no shell escape hatch, centralized+tested SSH POSIX quoting, and engine-
  enforced mode/guardrail/audit; the one `sh -c` (cron run-now) is the documented cron
  execution model passed as a single argv element, not an injection path. No critical or
  high findings.
- **S10.5 - Packaging**: static `x86_64`/`aarch64` binaries, AUR, `.deb`, `.rpm`,
  `cargo install`, `install.sh`, and release artifacts (checksums, signatures, SBOM).
  **Done.** Added the release pipeline (`.github/workflows/release.yml`, on `v*` tags):
  cross-compiles static musl `x86_64`/`aarch64` binaries (native + `cross`), tars them
  with the licenses and man page, builds `.deb`/`.rpm` from one `packaging/nfpm.yaml`,
  generates an SPDX SBOM (syft), writes `SHA256SUMS`, signs them with **keyless cosign
  (Sigstore/OIDC)** — the chosen approach, no private keys to manage — and publishes the
  GitHub Release. Added `LICENSE-MIT` + canonical `LICENSE-APACHE`, a real `systui.1` man
  page (matching the actual CLI), an `install.sh` that verifies checksum + cosign
  signature before installing, an AUR `PKGBUILD` (`systui-bin`), and `docs/INSTALL.md`
  (script / native packages / AUR / `cargo install` / verification). Validated locally:
  the `x86_64-unknown-linux-musl` build is fully static (`statically linked`, 5.8 MB
  stripped) and runs; `install.sh`/`PKGBUILD` pass syntax checks. The actual release run
  fires on the `v1.0` tag in S10.6. (Version bump to `1.0.0` is in S10.6.)
- **S10.6 - Docs & launch**: man page, README, examples, demo GIF, changelog, version
  bump to `1.0.0`, release CI/CD; final gates, merge `--no-ff` into `main` and tag
  `v1.0`.

## Definition of Done

Per `Product.md` section 16 and the roadmap:

- Stable local and remote (SSH) modes; the TUI does not crash on bad input, missing
  permissions, or unreachable hosts - it degrades to partial data.
- The full feature set works: smart dashboard, system/processes/services/logs/network/
  docker/crons/security/certs/databases/packages, reports (JSON/MD/HTML), profiles,
  read-only mode, audit log, action safety engine, and basic policies.
- Unit tests cover the correctness- and safety-critical layers; every command-output
  parser has fixture/golden-file tests; the action engine and policy evaluator are
  tested.
- Integration tests run the collectors against the main distros in containers and a
  systemd-capable environment, in CI.
- Parsers survive fuzzing without panicking; large logs stay responsive; a security
  review of the privileged/action paths is recorded with no open critical findings.
- SysTUI installs in under a minute via at least static binaries + one native package
  (`.deb`/`.rpm`/AUR) + `cargo install` + `install.sh`; artifacts ship with checksums,
  signatures and an SBOM.
- Documentation is complete: man page, README, examples, demo, changelog.
- Workspace version is `1.0.0`.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`
  and `cargo test --workspace` pass; the CI distro matrix and release pipeline are green
  before tagging `v1.0`.

## Risks & open questions

- **Distro matrix in CI cost/time.** Six distro containers plus a systemd environment
  can make CI slow. Mitigation: run the full matrix on release/PR-to-main and a reduced
  set on every push; cache aggressively.
- **systemd in CI.** Real systemd behavior is awkward to test in plain containers.
  Decide early between a privileged systemd container image and a lightweight VM; keep
  the non-systemd collectors testable without it so a single hard environment does not
  block the suite.
- **Static linking vs. glibc-only features.** musl static builds can change DNS/NSS and
  some libc behavior. Verify the network/DNS connectivity tools behave the same on the
  static binary as on a glibc build, or document the difference.
- **Signing key management.** Release signatures need a key and a place to store it that
  is not the repo. Decide on the signing approach (maintainer key vs. CI keyless/Sigstore
  style) before S10.5; never commit private keys.
- **Fuzzing scope creep.** Fuzzing every parser indefinitely is unbounded. Cap it:
  seed corpora from the existing fixtures, run a fixed-time fuzz pass in CI, and treat
  any panic as a release bug rather than chasing coverage numbers.
- **Demo GIF and "page web".** The roadmap mentions a web page; treat it as marketing
  polish, not a release blocker. The demo (GIF/asciinema) is in scope; a full website is
  optional and must not gate the tag.
- **AUR/`.deb`/`.rpm` maintenance burden.** Packaging metadata must track the version
  and dependencies. Generate as much as possible from the workspace manifest and the
  release CI to avoid drift across the formats.
