# SysTUI demo environment

Ways to populate the screens that are empty on a clean workstation
(Docker, Databases, failed Services, cron Jobs, the Fleet/Hosts grid), from
"works right now" to "full end-to-end". SysTUI is agentless: it inspects
whatever the transport reaches — your local machine, or a host over SSH.

| Screen | What it needs to show data |
|--------|----------------------------|
| Dashboard, System, Processes, Logs, Network, Security | Already populated from your local machine — just run the TUI. |
| **Services** (failed units) | A failed **system** unit → tier 1. |
| **Crons** (jobs) | A user crontab (needs `cron`) → tier 1 note. Timers already show. |
| **Docker**, **Databases** | Containers running → tier 2 (Docker). |
| **Hosts / Fleet**, SSH transport | A reachable host in the inventory → tier 3 (VM). |

Run the TUI with:

```sh
cargo run --release -p systui-cli           # local machine
```

---

## Tier 1 — local, no packages to install

**Failed service** (uses `sudo`, fully reversible):

```sh
demo-env/failing-service.sh up      # create a unit that fails on purpose
cargo run --release -p systui-cli   # tab 4 (Services): the unit + detail pane
demo-env/failing-service.sh down    # clean up
```

**Cron jobs**: the Crons tab already lists your systemd **timers**. To also see
the **jobs** table + schedule preview you need a cron daemon and a user crontab:

```sh
sudo pacman -S cronie && sudo systemctl enable --now cronie   # Arch
crontab -l 2>/dev/null > /tmp/systui-cron.bak || true         # back up first
( crontab -l 2>/dev/null; echo '*/5 * * * * echo demo' ) | crontab -
# … view in tab 9 (Crons) … then restore:
crontab /tmp/systui-cron.bak 2>/dev/null || crontab -r
```

> SysTUI can also add/edit/delete user-crontab entries from inside the TUI
> (`a`/`e`/`d`/`x` on the Crons tab) — it backs the crontab up before writing.

---

## Tier 2 — Docker / Databases / Network (needs Docker)

Install Docker, then bring up the demo stack:

```sh
sudo pacman -S docker docker-compose            # Arch
sudo systemctl enable --now docker
sudo usermod -aG docker "$USER"                 # then log out/in (or use sudo)

docker compose -f demo-env/docker-compose.yml up -d
cargo run --release -p systui-cli               # tab 8 (Docker)
docker compose -f demo-env/docker-compose.yml down
```

The stack runs nginx, redis, postgres plus two intentionally risky containers
(`demo-risky`: privileged + docker.sock + `:latest` + no mem limit; and
`demo-unhealthy`: a failing healthcheck). You should see:

- **Docker** — the container table with status dots, CPU/MEM, and **RISK** badges,
  plus the **Risk checks** panel and per-container detail.
- **Databases** — redis and postgres detected.
- **Network** — the published ports in the exposure map. They're bound to
  `127.0.0.1` (loopback, dim); change a mapping to `6379:6379` to demo a
  "risky" `0.0.0.0` listener.

---

## Tier 3 — Fleet / SSH (a disposable VM)

The Fleet/Hosts grid and the SSH transport need at least one reachable host. The
realistic test is a throwaway VM that has systemd, sshd, Docker, a couple of
failing units, crontabs and exposed ports — then point SysTUI at it:

```sh
systui ssh user@vm-ip            # drill into one host
systui fleet                     # the card grid over the inventory
```

Add hosts to `~/.config/systui/config.toml`:

```toml
[hosts.vm1]
host = "192.168.122.50"
user = "root"
tags = ["demo", "db"]

[hosts.local]
host = "localhost"               # works if sshd is running + key auth to localhost
user = "youruser"
```

Any VM tool works (libvirt/virt-install, multipass, a cloud instance). Auth and
host-key checking are delegated to your system `ssh` (`~/.ssh/config`, agent,
`known_hosts`), so if `ssh user@host` works, SysTUI works.
