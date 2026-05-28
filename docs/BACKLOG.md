# SysTUI — Post-v1.0 backlog

Candidate improvements gathered after the v1.0.0 release. Not scheduled yet; these are
v1.1+ ideas. See [`ROADMAP.md`](ROADMAP.md) "Out of scope until v1.1+" for the larger
deferred themes.

## UI — prototype parity for the System and Processes tabs — DONE

~~Both tabs render far more sparsely than the approved prototype.~~ **Completed.** The
**System** tab is now a multi-panel screen (`ui.rs::render_system`: identity, disks,
memory gauges, logged-in users) and the **Processes** tab has a detail side panel
(`render_process_detail`) plus a process-tree view (`build_process_tree`). The
`feat/tui-polish` branch further enriched both (System gained CPU model + virtualization).

## Feature — optional "expert console" (free-form shell), gated

Idea under consideration: a tab where the operator can type raw commands. **This
deliberately steps outside SysTUI's core guarantee** (no free-form commands; everything
through `CommandSpec` + the action engine's preview/risk/confirm/backup/audit). If built,
it must be designed so it does not quietly undermine that model:

- **Off by default**, enabled only by explicit config opt-in.
- **Disabled entirely in read-only mode** (it is a mutation surface by definition).
- **Every command audited** to the same local audit log as engine actions.
- A clear, visible boundary that the operator is leaving the safe/audited path.
- A **master password is an *access* gate, not the security mechanism** — once unlocked,
  commands run with the operator's full privileges, bypassing the engine. If added, store
  only a strong hash (e.g. argon2), never plaintext, and treat it as defense-in-depth on
  top of the controls above, not a substitute for them.

Recommendation: scope as an explicit, audited, opt-in v1.1 feature kept clearly separate
from the engine-mediated actions — not the default, and never presented as "safe".

## Feature ideas — real server-ops value (queued 2026-05-28)

Gathered while polishing the TUI. Ordered by value/effort. The first three reuse
collectors that already exist, so they are the cheapest high-impact wins.

### High value, low effort (extend existing collectors)

- **TLS/SSL certificate panel.** Certificates are already discovered and scored as
  findings (`cert_warning_days`, the v0.3 cert checks). Promote them to a first-class
  view: a sortable list of every cert (local files under `/etc/letsencrypt`,
  nginx/apache, plus probed `host:443`) with days-to-expiry, CN/SAN and issuer, colored
  by urgency. Expiry is the classic silent outage — make it impossible to miss.
- **Disk drill-down ("who's eating the disk").** `System` shows the global disk % only.
  Add an `ncdu`-style breakdown: top directories/files by size, fastest-growing logs,
  unrotated journald, package caches — i.e. *what to delete*, not just *how full*. New
  collector (`du`/`find` via `CommandSpec`), surfaced as a System sub-panel.
- **Security updates surfaced + reboot-required.** The packages collector already feeds
  the Dashboard UPDATES tile. Add: how many pending updates are *security* (apt/dnf/
  pacman/zypper security channels) and whether a reboot is required
  (`/var/run/reboot-required`, `needs-restarting -r`). Correlate with running services
  for "service X links a vulnerable lib, patch available".

### Higher effort / more differentiating

- **Auth & access panel (defensive).** Extend Security with the login picture: failed
  SSH attempts (journald / `lastb`) with repeat-offender IPs, current sessions,
  `authorized_keys` inventory, and users with a shell but no password. Builds on the
  existing `capabilities` (sudo) detection. This is the lively, AdGuardian-style
  event-list view.
- **Live mode + richer real-time sparklines.** Today only Dashboard CPU/RAM have
  sparklines. Extend the existing history/trends store to per-core CPU, disk I/O, and
  per-interface network throughput, plus a follow-mode log tail. Trends already persist,
  so 1h/24h history is mostly wiring.
- **Alerting / thresholds.** Simple rules ("disk >90%", "service failed", "cert <7d")
  that drive the tab badges (mechanism already exists in `tab_badge`) and feed the
  report. Turns SysTUI from a viewer into a watcher.
- **Config/state drift.** Snapshot host state and diff across runs ("this port wasn't
  open yesterday"). The local snapshot store from v0.8.4 is the substrate.

### Extensions to existing tabs

- **Network:** established connections with reverse-DNS, per-process bandwidth
  (`nethogs`-style).
- **Docker:** live per-container stats, healthcheck status, reclaimable space from
  orphaned images/volumes, compose awareness.
- **Services:** dependency view (`systemctl list-dependencies`) and boot timing
  (`systemd-analyze blame`) for slow-boot diagnosis.
- **Logs:** known-pattern detection (OOM killer, segfault, disk errors) and log-spike
  alerts (a service suddenly logging 100×).

## Docs

- Demo GIF / asciinema for the README (the one optional v1.0 docs item left outstanding).

## Dependencies

- Bump `ratatui` when a release clears the transitive `paste` (unmaintained) and `lru`
  (unsound) advisories flagged by `cargo audit` (see [`AUDIT-2026-05.md`](AUDIT-2026-05.md)).
- Consider a `cargo audit` / `cargo deny` step in CI to track new advisories.
