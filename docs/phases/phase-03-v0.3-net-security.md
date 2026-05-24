# Phase 3 — v0.3 Network & security

> First session (S3.1) deliverable. Freezes scope and decisions for v0.3 before
> coding. See [`../ROADMAP.md`](../ROADMAP.md) and [`../METHODOLOGY.md`](../METHODOLOGY.md).
> Built on `release/v0.3` (from `main` after v0.2); tag `v0.3` when the DoD is met.

## Goal

Make SysTUI genuinely *differentiating*: answer "what is exposed on this host, which
process exposes it, and what should I worry about first?" — with evidence. This adds
the network module, the **exposure map**, the first **security findings**, and TLS
certificate checks (`Product.md` §4.6, §4.10, §4.12, §7.2, §7.4, §8 v0.3).

Everything stays read-only this phase: we detect, explain and prioritize. Remediation
actions are out of scope (the action engine exists, but v0.3 only inspects).

## In scope

- **Network collectors**: interfaces + IPs, routes, DNS servers, listening ports,
  active connections (by state), each listener's owning process.
- **Correlation**: port → process (pid/name) → systemd unit where determinable.
- **Exposure map**: classify each listener by bind scope (loopback / external) and
  port sensitivity; assign a risk level with evidence.
- **Connectivity tools**: ping, DNS lookup, TCP connect test to `host:port`.
- **Finding model**: shared `Finding { id, severity, title, evidence, impact,
  recommendation, module, status }` (`Product.md` §9) with `FindingStatus`.
- **Security checks** (read-only, evidence-based): SSH root login, SSH password auth,
  sudo users, recent failed SSH logins, firewall presence (ufw/firewalld/nft/iptables),
  sensitive exposed ports, world-writable/incorrect perms on critical files, docker
  socket exposure, a few SUID binaries.
- **Certificates**: discover certs in common paths and read remote `host:443`; check
  expiry, CN/SAN, issuer, self-signed.
- **Security tab + dashboard hook**: a prioritized findings list with severities; a
  security-score / findings count on the dashboard.

## Out of scope (deferred)

- Any firewall/ssh *remediation* actions → later (read-only detection only now).
- Full CIS compliance → never promised; "CIS-inspired checks" only (`Product.md` §4.10).
- Docker module proper, crons → phase 4.
- Finding lifecycle UI (accept/ignore/exception) → phase 9 (policies); model the
  status now but the workflow comes later.
- SSH/remote transport → phase 5.

## Key decisions

- **Data sources (agentless):** `ss -tulpn` (listeners + owning process), `ss -tan`
  (connections by state), `ip -j addr` / `ip -j route` (JSON where available, text
  fallback), `/etc/resolv.conf` (DNS). Sensitive config via file reads:
  `/etc/ssh/sshd_config`, `/etc/passwd`, `/etc/sudoers`(+`.d`). Firewall presence by
  probing `ufw status` / `firewall-cmd` / `nft list ruleset` / `iptables -S`.
  Everything via `CommandSpec`/`read_file`.
- **Port → service correlation:** `ss -p` gives `pid,name`; map pid → unit via
  `/proc/<pid>/cgroup` (systemd slice) rather than spawning `systemctl status`.
- **Exposure classification:** bind `0.0.0.0`/`::` (or non-loopback) = externally
  reachable; sensitive ports = {22, 3306, 5432, 6379, 9200, 27017, 11211, 5984, ...}.
  Risk = external + sensitive → High/Critical (e.g. Redis on 0.0.0.0 = Critical);
  external + non-sensitive = Info/Low; loopback = OK. Each entry carries evidence.
- **Finding model in core** (shared by security and network/exposure); checks live in
  `systui-security`. Findings are produced by pure functions over collected inputs so
  they are fixture-testable. Severity reuses `core::Severity`.
- **Certificates:** prefer the `openssl` CLI via transport (`x509 -noout -enddate
  -subject -issuer` for files; `s_client -connect host:443` piped to `x509` for
  remote) over pulling a Rust X.509 stack now. Parse dates to compute days-to-expiry
  against `config.security.cert_expiry_warning_days`. If `openssl` is absent, degrade.
- **Read-only safety:** no new actions; the security module only reads. Missing
  permissions (e.g. unreadable `sshd_config`) degrade to partial data, never crash.
- **Parsers tested with fixtures:** `ss`, `ip`, firewall and `openssl` outputs all get
  captured fixtures (`Product.md` §12).

## Sessions

- **S3.1 — Context** *(this file)*. **Done.**
- **S3.2 — Network collectors**: interfaces/IPs/routes/DNS, listening ports and
  active connections with owning process, fixture-tested. **Done.** `NetworkCollector`
  in `systui-collectors` produces a `NetworkSnapshot { interfaces, routes, dns,
  listeners, connections }` via `ip -j addr|route` (JSON, with a text fallback),
  `/etc/resolv.conf` and `ss -tulpn`/`ss -tan`; each source degrades to empty
  independently. Listeners carry the owning `ProcessRef` from `ss -p`.
- **S3.3 — Correlation**: port → process → systemd unit (via `/proc/<pid>/cgroup`).
  **Done.** `correlate_units` fills each `Listener.unit` from the leaf-most
  `.service`/`.scope` segment of the owning pid's cgroup (v1 + v2); PIDs read once,
  unreadable cgroups degrade to `None`. `NetworkCollector` runs it automatically.
- **S3.4 — Exposure map**: classify listeners (scope + sensitivity) into risk-ranked
  exposure entries with evidence. **Done.** `exposure_map` in `systui-collectors`
  maps each `Listener` to an `ExposureEntry { scope, sensitive_service, severity,
  evidence }`: loopback binds = `Info`; external + sensitive port = `High`
  (ssh/mysql/postgres) or `Critical` (redis/mongodb/memcached/elasticsearch/couchdb);
  external + non-sensitive = `Low`. Reuses `core::Severity`; sorted worst-first.
- **S3.5 — Connectivity tools**: ping, DNS lookup, TCP connect test. **Done.**
  `connectivity` module: `ping` (parses transmitted/received/loss + rtt min/avg/max),
  `dns_lookup` (via `getent ahosts`, the portable NSS resolver) and `tcp_connect`
  (via `nc -z`, reading the exit code). All run a single timeout-bounded
  `CommandSpec` through the transport; read-only diagnostics, fixture-tested.
- **S3.6 — Findings & security checks**: `Finding` model + the initial posture checks.
- **S3.7 — Certificates**: local discovery + remote `host:443`, expiry/CN/issuer checks.
- **S3.8 — Security tab + polish**: Network/Security tabs render exposure + findings;
  dashboard shows a findings/exposure summary → **tag v0.3**.

## Definition of Done

- The Network tab shows interfaces, listening ports with owning process, and active
  connections; the exposure map ranks externally reachable, sensitive ports with
  evidence.
- The Security tab shows a prioritized, evidence-based findings list (SSH, sudo,
  failed logins, firewall, sensitive ports, file perms, docker socket, certs).
- Connectivity tools (ping / DNS / TCP connect) work from the TUI.
- Certificate expiry is detected for local and a given remote `host:443`.
- All new parsers have fixture tests; `cargo fmt --check`, `cargo clippy -D warnings`
  and `cargo test --workspace` pass.

## Risks & open questions

- **`ss`/`ip` output & flags vary** by distro/version; prefer JSON (`-j`) where
  supported and capture text-format fixtures as fallback. busybox lacks `ss -p`.
- **Process → unit mapping** from cgroup paths differs (cgroup v1 vs v2); parse
  defensively and degrade to "process only" when unknown.
- **`openssl` may be absent**; certificate checks must degrade, and remote
  `s_client` needs a timeout to avoid hangs.
- **Failed-login source** varies (`journalctl _SYSTEMD_UNIT=ssh*` vs `/var/log/auth.log`
  vs `lastb`); pick the most portable and note permission requirements.
- **Finding IDs/stability:** define a stable id scheme now so phase-9 exceptions can
  reference findings later.
