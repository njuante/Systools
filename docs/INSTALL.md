# Installing SysTUI

SysTUI ships as a single static Linux binary (`x86_64` and `aarch64`). Pick whichever
route fits your system; all of them install the same `systui` command.

## Quick install (script)

```sh
curl -fsSL https://raw.githubusercontent.com/njuante/Systools/main/install.sh | sh
```

The script detects your architecture, downloads the matching release tarball, verifies
its SHA256 (and its keyless Sigstore signature when `cosign` is installed), then installs
to `/usr/local/bin` (or `~/.local/bin` if that is not writable). Override with
`SYSTUI_VERSION`, `SYSTUI_BINDIR` or `SYSTUI_REPO`.

## Native packages

Download the `.deb` or `.rpm` for your architecture from the
[latest release](https://github.com/njuante/Systools/releases/latest):

```sh
# Debian / Ubuntu
sudo dpkg -i systui_1.0.0_amd64.deb

# Fedora / RHEL / Rocky / Alma
sudo rpm -i systui-1.0.0-1.x86_64.rpm
```

## Arch Linux (AUR)

```sh
paru -S systui-bin     # or: yay -S systui-bin
```

The `PKGBUILD` lives in [`packaging/aur/`](../packaging/aur/PKGBUILD).

## From source (cargo)

```sh
cargo install --git https://github.com/njuante/Systools systui-cli
```

This builds the `systui` binary from the `systui-cli` crate. Requires the Rust toolchain
(edition 2024, see `rust-version` in `Cargo.toml`).

## Verifying artifacts

Every release ships `SHA256SUMS`, a keyless Sigstore signature
(`SHA256SUMS.cosign.bundle`) and an SPDX SBOM (`systui.spdx.json`).

```sh
# Checksums
sha256sum -c SHA256SUMS

# Signature (no key needed — verifies the GitHub Actions OIDC identity)
cosign verify-blob \
  --bundle SHA256SUMS.cosign.bundle \
  --certificate-identity-regexp "https://github.com/njuante/Systools" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
  SHA256SUMS
```

## How releases are built

Pushing a `vX.Y.Z` tag triggers [`release.yml`](../.github/workflows/release.yml), which
cross-compiles the static musl binaries, packages them (tar.gz/.deb/.rpm via
[`nfpm`](../packaging/nfpm.yaml)), generates checksums + SBOM, signs the checksums with
cosign keyless, and publishes the GitHub Release.
