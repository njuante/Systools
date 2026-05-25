# SysTUI — Security review (v1.0, phase 10 / S10.4)

Date: 2026-05-25. Scope: the command-execution, privilege and action paths, and the
parser hardening surface. This is the release security review required by
`Product.md` §15 and the phase-10 DoD. It records what was audited, the result, and
the tests that now pin each invariant.

## 1. Threat model (what this review cares about)

SysTUI runs commands on hosts the operator administers, locally and over SSH. The
relevant risks are therefore:

1. **Command injection** — a value (unit name, PID, path, host, cron line) reaching a
   shell and being re-parsed as code.
2. **Privilege misuse** — a mutation running with more privilege than the mode allows,
   or without confirmation/audit.
3. **Parser crashes on hostile output** — a malformed/huge/adversarial command output
   panicking the process instead of degrading.

Out of scope (by design, deferred past v1.0): automatic remediation, secret storage,
network-facing services. SysTUI opens no listening sockets and stores no credentials.

## 2. The core invariant: no free-form command strings

**Every command is a typed `CommandSpec { program, args, requires_privilege }`** built
in code; the UI never executes commands, it requests `Action`s and the engine decides.
Audit findings:

- **No `std::process::Command` / `Command::new` outside the transport layer.** All
  execution funnels through `Transport` (`Local`/`Ssh`/`Mock`) over `CommandSpec`.
  Verified by grep across `crates/*/src` — the only hits are inside `systui-transport`.
- **No shell escape hatch.** There is no `CommandSpec::shell`, no `exec_str`, no raw
  string execution API. Arguments are passed as an argv vector, so on the local
  transport the kernel `execve`s the program directly with no shell in the path.
- **SSH quoting is centralized and tested.** The remote shell *does* re-parse what we
  send, so `build_remote_command` turns the `CommandSpec` into a single, strictly
  POSIX-quoted string in exactly one place (`quote_arg`: safe tokens pass through,
  everything else is single-quoted with `'` escaped as `'\''`). The injection case
  (`/etc/passwd; rm -rf /` as an argument) is covered by a dedicated test that asserts
  the metacharacters are quoted, not interpreted.

### Deliberate, analyzed exception: cron "run now"

`systui-actions/src/cron.rs` builds `CommandSpec::new("sh").args(["-c", &self.command])`.
This is intentional and safe:

- `self.command` is the **existing crontab command line**, reproduced exactly as cron
  itself would run it (`sh -c <line>`) — it is not free-form input typed into SysTUI.
- It is passed as a **single argv element**, so no SysTUI-side interpolation occurs;
  over SSH it is quoted as one token by `quote_arg`. The shell here is cron's documented
  execution model, not an injection vector we introduce.

This is the only `sh -c` in the codebase and is documented at the call site.

## 3. Privilege and action safety

The action engine (`systui-actions/src/engine.rs`) is the single pipeline every
mutation passes through: `guardrail → mode/permission → risk → preview → confirmation
→ execute → verify → audit`. Reviewed and confirmed:

- **Mode gating is enforced on both `plan` and `execute`** (not just advisory in the
  UI): read-only rejects all mutations; safe mode blocks high-risk ops; risky ops
  require a typed, case/whitespace-insensitive confirmation phrase. Re-checked on the
  execute path so a stale plan cannot bypass it. (Tests added in S10.2.)
- **Guardrails are hard blocks**, re-checked on execute — e.g. signalling PID 1 is
  rejected regardless of mode.
- **Every run is audited** with a status (`Success`/`Failure`/`Rejected`), including
  rejected and failed attempts, so the audit trail is not just a success log.

## 4. Parser hardening (S10.4)

Command output is trusted today but can be truncated, localized, busybox-flavored or
hostile. The release adds property-based robustness tests (`systui-testkit::fuzz`)
that throw both fully-arbitrary text (incl. control bytes, newlines, oversized
integers) and table-shaped adversarial input at the parsers, asserting the one
invariant that must always hold: **parsing never panics — it returns empty/partial
data.** Coverage:

- collectors: system, network, service, logs, packages, firewall, cron (400 cases each);
- security: certificates, sshd_config, failed-login counting, `stat`/iptables, sudoers.

No panics were found. The large-journal path is additionally covered by a
correctness-at-scale test (100k entries) and a criterion throughput benchmark
(`benches/log_parse.rs`), which shows linear scaling (~0.9M entries/s, ~220 ms for
200k lines) — no quadratic blow-up.

## 5. Findings & conclusion

No critical or high findings. The no-free-form-command invariant holds end to end; the
one shell use (cron run-now) is the documented cron execution model, not an injection
path. Privilege gating and auditing are enforced in the engine, not the UI. Parsers
degrade rather than crash on adversarial input, now pinned by property tests.

Residual notes (not release blockers):

- The SSH transport shells out to the system `ssh` client; its safety depends on the
  operator's `known_hosts`/key configuration (by design — SysTUI reuses existing SSH
  trust). Non-interactive `BatchMode=yes` avoids password-prompt hangs.
- Parser robustness is proven against generated input, not an exhaustive corpus; the
  property tests run a fixed number of cases per gate. New parsers must add a fuzz arm
  (see [`TESTING.md`](TESTING.md)).
