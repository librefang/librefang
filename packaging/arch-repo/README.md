# Arch pacman repository

The project-maintained binary repository for Arch Linux, published to Cloudflare R2 behind `packages.librefang.ai`.
Install LibreFang with `pacman -Syu` and track every release through normal system updates.

This is distinct from the AUR packages under `packaging/aur/`.
It exists because AUR account registration was closed with no reopening date (see #6334), which blocks the AUR automation in #6341.
This repository ships the same release-pinned binary packages directly, with no AUR account required.
The two are complementary: when AUR registration reopens, #6341 publishes the AUR `-bin` packages for `yay` users, while this repository keeps serving `pacman` users.

## Installing (users)

```sh
# 1. Import the LibreFang packaging public key and locally sign it so pacman
#    will trust packages signed by it.
curl -fsSL https://packages.librefang.ai/librefang.gpg -o /tmp/librefang.gpg
sudo pacman-key --add /tmp/librefang.gpg
sudo pacman-key --finger packaging@librefang.ai      # confirm the fingerprint, then:
sudo pacman-key --lsign-key <FINGERPRINT-printed-above>

# 2. Add the repository to /etc/pacman.conf (append at the end):
#
#      [librefang]
#      Server = https://packages.librefang.ai/arch/$arch
#
# 3. Sync and install. The CLI/daemon and the desktop app:
sudo pacman -Syu
sudo pacman -S librefang-bin librefang-desktop-bin
```

`$arch` is pacman's own variable — leave it literal in `pacman.conf`; pacman expands it to `x86_64`.
Both the database and every package are GPG-signed, so the default `SigLevel` (inherited from `[options]`) verifies them once the key above is locally signed.
Do **not** set `SigLevel = Never` — that disables the verification this repository exists to provide.

Available packages:

- `librefang-bin` — CLI, daemon, HTTP API, and dashboard on port 4545.
- `librefang-desktop-bin` — native desktop launcher.
- `librefang-docker` — Docker-backed systemd service pinned to the release tag.

## How it works (CI)

`release.yml`'s `publish_arch_repo` job runs `publish-arch-repo.sh` inside an `archlinux:base-devel` container on every `v*` tag (and on a `channel=current` re-publish).
The script:

1. Reuses the committed PKGBUILDs under `packaging/aur/<package>/` as the source of truth, deriving only the per-release values — `pkgver` (encoding the tag's first `-` as `_`), `pkgrel=1`, the desktop bundle version (read off the actual `LibreFang_<ver>_amd64.deb` asset name), the pinned `ghcr.io/librefang/librefang:<version>` tag — then regenerates `sha256sums` with `updpkgsums`.
2. Builds and GPG-signs each package with `makepkg --sign` (no Rust compile — these repackage the prebuilt release artifacts).
3. Folds every package into one shared, signed pacman database with `repo-add --sign`, pulling the existing database from R2 first so the update is incremental.
4. Uploads the packages, signatures, database, and the public key to R2.
5. Prunes old package files beyond the newest `RETAIN` (default 5) per package — best-effort, kept only for manual `pacman -U <url>` downgrades; the database always points at the latest build.

The job degrades to a no-op with a notice until the maintainer configures the signing key and R2 credentials, so it is safe to merge before the secrets exist.

Object storage has no symlinks, so `librefang.db` / `librefang.files` (which `repo-add` writes as symlinks to their `.tar.gz`) are materialised as real objects before upload.

## One-time maintainer bootstrap

1. Create the signing key **offline** — a primary key for identity plus a passphrase-less signing subkey for CI — and an R2 bucket bound to `packages.librefang.ai`.
   Exact commands, formats, and rotation policy are in `.github/SECRETS.md` (`Arch pacman repository` section).
2. Add the secrets: `ARCH_REPO_GPG_PRIVATE_KEY`, `ARCH_REPO_GPG_KEY_ID`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY` (`CLOUDFLARE_ACCOUNT_ID` is reused from the Workers deploys).
3. Validate end-to-end with `workflow_dispatch` on `release.yml` (`channel=current`, `tag=<an existing release tag>`) before the next real release.
   The first run cold-starts the database; subsequent runs update it incrementally.

The committed `pkgver` / `sha256sums` under `packaging/aur/` are a working baseline for local `makepkg`; the release-correct values are derived into R2 on each tag and are never committed.
