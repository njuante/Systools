# SysTUI — Testing & fixture convention

How SysTUI is tested, and the rules for the fixture/golden-file corpus that proves
the command-output parsers behave across distros. The quality gates themselves live in
[`METHODOLOGY.md`](METHODOLOGY.md) §6; this document covers *what* we test and *how the
fixtures are organized* so adding a new distro later is mechanical.

---

## 1. Layers and what each one proves

| Layer | Where | What the tests prove |
|-------|-------|----------------------|
| **Parsers** | `systui-collectors`, `systui-security` | Command output → typed data, per distro flavor. Happy path **and** empty/malformed/partial input must not panic. |
| **Action engine** | `systui-actions` | The safety pipeline: guardrail → mode → risk → confirmation → execute → verify → audit status. |
| **Policy evaluator** | `systui-security::policy` | Collected facts + policy → stable `policy.*` findings; unknown areas become `policy.partial.*`, never silent compliance. |
| **Findings / severity** | `systui-security`, `systui-core` | Severity ranking and evidence are deterministic. |
| **Render** | `systui-ui` | `render(&App)` is a pure function; `TestBackend` snapshots stay stable. |
| **Transport** | `systui-transport` | `MockTransport` matches the `Local`/`Ssh` contract used everywhere else. |

The methodology rule is absolute: **a parser that reads command output without a
fixture test is a release bug.**

---

## 2. Fixture corpus

Fixtures are real command output captured verbatim. They live next to the crate that
parses them:

```
crates/systui-collectors/fixtures/
crates/systui-security/fixtures/
```

### Loading

Parsers are pure functions over `&str`, so fixtures are embedded at compile time and
fed directly to the parser:

```rust
let ifaces = parse_ip_addr_json(include_str!("../fixtures/ip-addr.json")).unwrap();
```

For an end-to-end collector test, the same fixture is served through `MockTransport`
so the command-dispatch path is exercised too:

```rust
let transport = MockTransport::new()
    .with_stdout("ip -j addr", include_str!("../fixtures/ip-addr.json"))
    .with_stdout("ss -tulpn", include_str!("../fixtures/ss-tulpn.txt"));
```

Use `include_str!` for text, `include_bytes!` when the collector consumes raw bytes
(e.g. `/proc` reads via `MockTransport::with_bytes`).

### Naming

`<tool>-<variant>.<ext>` — the producing command first, a variant when one tool has
several shapes, the natural extension last:

- `ip-addr.json` / `ip-addr.txt` — same data, JSON vs. text output of `ip addr`.
- `ss-tulpn.txt`, `ss-tan.txt` — different `ss` invocations.
- `systemctl-failed.txt`, `systemctl-list-units.txt`, `systemctl-show.txt`.
- `sshd_config-hardened.txt` / `sshd_config-insecure.txt` — opposite-state pairs.
- `cgroup-v1-service.txt` / `cgroup-v2-service.txt` — same fact across kernel versions.

Keep fixtures minimal but representative: enough rows to exercise every branch
(header skipping, multiple entries, the pseudo-filesystem the parser filters out),
trimmed of noise that does not affect parsing.

---

## 3. Adding support for a new distro

The parsers are distro-agnostic by design, so a new distro is added by **capturing its
output as fixtures**, not by branching code:

1. On the target distro, run the command the collector uses and save the output to
   `crates/<crate>/fixtures/<tool>-<distro>.<ext>`.
2. Add a test that feeds the new fixture to the parser and asserts the typed result.
3. If the new output reveals a real format difference, fix the parser and keep **both**
   fixtures so neither distro regresses.

Capturing fixtures from a real distro container (see the integration matrix in
`phase-10` S10.3) keeps them honest.

---

## 4. Robustness expectations (parsers)

Every parser must tolerate, without panicking:

- **empty input** — returns an empty collection / `None`, not a crash;
- **a header with no rows** — returns empty;
- **a malformed or truncated line** — skips it (or records partial data) and parses the rest;
- **unexpected extra columns / fields** — ignored, not fatal.

These are unit-tested per parser and stress-tested by the fuzzing pass in `phase-10`
S10.4; the two are complementary — fixtures pin known formats, fuzzing finds the
unknown ones.

---

## 5. Running the suite

```sh
cargo test --workspace                 # everything
cargo test -p systui-collectors        # one crate
cargo test -p systui-actions engine    # one module's tests
```

All three gates must be green before any merge to a release branch or `main`:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
