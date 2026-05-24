# Phase 5 — v0.5 Remote SSH

> First session (S5.1) deliverable. Freezes scope and decisions for v0.5 before
> coding. See [`../ROADMAP.md`](../ROADMAP.md) and [`../METHODOLOGY.md`](../METHODOLOGY.md).
> Built on `release/v0.5` (from `main` after v0.4); tag `v0.5` when the DoD is met.

## Goal

Turn SysTUI into a real remote administration tool. Everything the app already
does locally — system, processes, services, logs, network, docker, crons,
security and certificate checks — must work against a remote host over SSH, with
an **identical UI**, simply by choosing a different `Transport`. The whole point
of the v0.0 architecture (every module talks to a `Transport`, never the OS) pays
off here: adding `SshTransport` should light up every existing module remotely
without touching any of them (`Product.md` §2, §8 v0.5, "Fase 9 — SSH remoto").

It stays **agentless**: nothing is installed on the target; we connect over SSH,
run the same `CommandSpec`s and read the same files we read locally.

## In scope

- **`SshTransport`** implementing the full `systui_core::Transport` contract
  (`run`, `read_file`, `file_exists`, `list_dir`, `label`) against a remote host:
  key authentication, SSH-agent authentication, custom port, and connect/command
  timeouts. stdin on a `CommandSpec` is forwarded to the remote process.
- **Target resolution**: `systui ssh user@host` (ad-hoc) and `systui ssh <host-id>`
  resolving an inventory entry `[hosts.<id>]` (host / user / port / `read_only`)
  from the existing `Config`. The CLI `Ssh { target }` stub is wired up.
- **Host profiles & known hosts**: reuse the inventory `Host` model already in
  `systui-core::config`; rely on the system's `known_hosts`/`~/.ssh/config` for
  host-key verification. Basic reconnect on a dropped connection.
- **Remote permission detection**: probe what the connected user can actually do
  (e.g. `id`, non-interactive `sudo -n`) so the session degrades to read-only or
  "partial data" instead of failing opaquely.
- **Remote read-only enforcement**: identical to local — the action engine gates
  mutations by `ExecutionMode`; per-host `read_only = true` forces it regardless
  of CLI flags.
- **Local/remote parity**: verify every existing module works over SSH and fix
  any accidental local-only assumptions, keeping the UI byte-for-byte identical
  apart from the host label.

## Out of scope (deferred)

- **Native Rust SSH backend** (e.g. `russh`): not this phase — but the design must
  let it replace the OpenSSH backend later **without the rest of the app noticing**
  (`Product.md` "Fase 9", técnica). See Key decisions.
- **Interactive / password authentication**: v0.5 is non-interactive (keys + agent
  only). A host that would prompt for a password fails fast with a clear message.
- **Fleet / multi-host**: host selector, tags, groups, favourites, last-used,
  concurrent checks and host comparison are **phase 8** (`Product.md` §4.16). v0.5
  resolves and connects to exactly one host.
- **Expected-state policies** (`[policies.*]`): phase 9.
- **SFTP / file uploads / file writes**: we read files by running a command
  (`cat`), and actions run commands — we never transfer files. No write path.
- **Jump hosts / bastions beyond `~/.ssh/config`**: whatever the system ssh client
  already does via `ProxyJump`/config works for free; we add nothing extra.

## Key decisions

- **Backend: the system OpenSSH client, wrapped behind `SshTransport`.** We invoke
  `ssh` (in `systui-transport::ssh`) rather than embedding an SSH stack. Rationale:
  it is the fastest path to broad compatibility and it reuses, for free, everything
  operators already trust — key auth, **ssh-agent**, `known_hosts`, `~/.ssh/config`,
  `ProxyJump`. `Product.md` "Fase 9" explicitly green-lights starting here. The
  single-binary purity cost is **accepted for v0.5** and contained: all SSH
  specifics live behind the `Transport` trait, so a native Rust backend can replace
  this later and *the rest of the app never knows* (the stated invariant: "lo
  importante es que el resto de la app no se entere").
- **One audited shell-quoting layer at the SSH boundary.** OpenSSH concatenates the
  remote command and the target's login shell re-parses it, so each `CommandSpec`
  `{program, args}` is turned into a single, strictly POSIX-quoted command string in
  exactly one place. This preserves the project's "no unsafe interpolation"
  guarantee across the network and is **fixture-tested** — it is the one spot where
  a quoting bug would reintroduce injection, so it gets dedicated tests.
- **Non-interactive by default**: pass `-o BatchMode=yes` and `-o ConnectTimeout=N`
  so authentication never blocks on a prompt; a host without working key/agent auth
  returns a typed error fast. Command timeouts use the existing `CommandSpec.timeout`
  plus a tokio timeout, mirroring `LocalTransport`.
- **Error mapping mirrors `LocalTransport`.** Connection failures, auth failures and
  timeouts map to `CoreError::Transport` / `CoreError::Timeout` /
  `CoreError::PermissionDenied`; a remote "no such file" maps to
  `CoreError::FileNotFound`, so collectors degrade to partial data exactly as they
  do locally.
- **Remote `Transport` methods via small portable commands**: `read_file` → `cat --`,
  `file_exists` → `test -e`, `list_dir` → a minimal, fixture-tested listing command.
  These are the only remote-specific parsers and follow the §12 fixture rule.
- **Connection reuse**: spawning a fresh `ssh` per command is correct but slow over
  high-latency links; OpenSSH `ControlMaster`/`ControlPersist` multiplexing is the
  intended optimisation (evaluated in S5.2/S5.3), kept behind the transport.
- **No module changes.** Collectors, the action engine, security/docker/cron checks
  already depend only on `Transport`. Parity is a verification + bug-fix session, not
  a rewrite. The UI differs only by the host label and the chosen transport.

## Sessions

- **S5.1 — Context** *(this file)*.
- **S5.2 — `SshTransport`**: implement the `Transport` contract over the OpenSSH
  client — key auth, ssh-agent, custom port, connect/command timeouts, stdin
  forwarding, the quoting layer and error mapping. Fixture/unit-tested.
- **S5.3 — Host profiles**: resolve `user@host` and inventory host ids; reuse
  `known_hosts`/`~/.ssh/config`; basic reconnect; wire the `systui ssh <target>`
  CLI path to launch the TUI with an `SshTransport`.
- **S5.4 — Remote read-only + permission detection**: probe the remote user's
  capabilities, degrade the execution mode / surface "partial data", and enforce
  read-only remotely exactly as locally.
- **S5.5 — Local/remote parity** + polish: verify every module over SSH, fix any
  local-only assumptions, confirm the UI is identical → **tag v0.5** (final session:
  merge `--no-ff` into `main` + tag).
  **Done.** Parity audit confirmed **no module touches the host outside the
  `Transport`** (collectors, security, actions, UI and report contain no direct
  `std::fs`/`std::process`/`LocalTransport` use; the only `std::env` is the local
  operator name for the audit record), so every module works over `SshTransport`
  with zero module changes — verified at the SSH boundary by a round-trip test of
  the real collector commands (`ps`/`systemctl`/`ss`/`docker`/`journalctl`/`stat`).
  Read-only is enforced by the action engine identically for any transport, and
  permission limits already surface as partial data. Polish: an actionable SSH
  connection-failure message (key/agent/known_hosts hint) and a "Connecting to …"
  notice before the TUI takes the screen.

## Definition of Done

- `systui ssh user@host` and `systui ssh <host-id>` open the same TUI as local,
  operating on the remote host.
- `SshTransport` implements `run` / `read_file` / `file_exists` / `list_dir` with
  key + agent auth, a custom port and timeouts, mapping all failures to typed
  `CoreError`s and degrading gracefully (never hanging, never crashing).
- Every existing module (system, processes, services, logs, network, docker, crons,
  security, certificates) works over SSH **with no module-level changes**.
- Remote read-only mode is enforced identically to local; per-host `read_only`
  profiles are honoured.
- Remote permission limits surface as partial data, not crashes.
- The UI is identical local vs remote apart from the host label.
- `cargo fmt --check`, `cargo clippy -D warnings` and `cargo test --workspace`
  pass; the quoting layer and any remote-listing parser have fixture tests.

## Risks & open questions

- **Quoting at the boundary is the sharp edge.** The remote shell re-parses the
  command, so the POSIX-quoting layer must be exhaustively fixture-tested; a single
  gap reintroduces shell injection. This is the highest-risk piece of the phase.
- **Remote `list_dir`/`file_exists` portability**: the chosen listing command must
  behave on mainstream Linux targets and map errors correctly (not-found vs
  permission-denied vs connection-failed). Capture fixtures from real output.
- **`BatchMode` excludes password-only hosts.** v0.5 requires key/agent auth; this
  must be documented so a "connection failed" message is understood, not surprising.
- **Privileged commands remotely**: `docker` and other root-only commands may need
  `sudo` on the target; remote privilege handling is tied to permission detection
  (S5.4) and may limit what mutations are possible without interactive sudo.
- **Per-command `ssh` spawn latency**: acceptable for correctness first; revisit
  with ControlMaster multiplexing if the dashboard refresh feels slow over WAN.
- **Single-binary tension**: depending on the system `ssh` client is a deliberate,
  temporary trade-off; the native-backend migration path must stay open and is the
  reason all SSH details are sealed behind the `Transport` trait.
