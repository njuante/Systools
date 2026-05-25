# SysTUI

**Fast, agentless TUI for Linux server administration.**

SysTUI is a terminal user interface that inspects and safely operates Linux servers —
locally or over SSH — without installing anything on the remote host. It does not just
show metrics: it **detects, explains, correlates, acts safely, and reports**. A failed
service is connected to its logs, its process, the port it exposes, the risk that
creates, and the action that fixes it.

```
systui                # Hosts grid: the local machine + your SSH inventory
systui local          # inspect this machine
systui ssh web-01     # inspect a remote host (inventory id or user@host)
systui fleet --tag prod
systui report --host db-01 --format html -o db-01.html
```

## Why SysTUI

- **Agentless.** Everything runs over the local shell or the system `ssh` client —
  nothing is installed on the target, and existing `known_hosts`/`ssh-agent`/`~/.ssh/config`
  trust is reused.
- **Safe by default.** Read-only unless you act. Every change passes one engine:
  guardrail → mode/permission → risk → preview → typed confirmation → backup → execute →
  verify → audit. No free-form shell commands are ever constructed.
- **Evidence-based.** Findings carry the evidence, the risk and a recommendation — not
  just "SSH is insecure".
- **Single static binary.** One download, no runtime dependencies, `x86_64` and `aarch64`.

## Features

System & hardware, processes, systemd services, journald logs, network & exposure map,
Docker, cron & timers, security posture, TLS certificates, databases, packages/updates,
a fleet overview across many hosts, expected-state **policies** with drift findings, and
reports in Markdown / JSON / HTML.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/njuante/Systools/main/install.sh | sh
```

Native packages (`.deb`/`.rpm`), AUR (`systui-bin`), `cargo install`, and artifact
verification (checksums, keyless Sigstore signature, SBOM) are documented in
[`docs/INSTALL.md`](docs/INSTALL.md).

## Safety model

SysTUI runs in three modes — **read-only**, **safe actions**, **privileged** — and the
action engine enforces them: read-only rejects every mutation, safe mode blocks
high-risk operations, and risky actions require a typed confirmation. Every attempt
(success, failure or rejection) is written to a local audit log. See
[`docs/SECURITY-REVIEW.md`](docs/SECURITY-REVIEW.md).

## Build from source

```sh
cargo build --release        # requires the Rust toolchain (edition 2024)
cargo run -p systui-cli
```

Quality gates: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`. Per-distro integration tests run in CI; see
[`docs/TESTING.md`](docs/TESTING.md).

## Documentation

- [Roadmap](docs/ROADMAP.md) · [Methodology](docs/METHODOLOGY.md)
- [Install](docs/INSTALL.md) · [Testing](docs/TESTING.md) · [Security review](docs/SECURITY-REVIEW.md)
- [Changelog](CHANGELOG.md)

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
