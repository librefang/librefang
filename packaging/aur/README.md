# AUR packaging

This directory holds upstream-maintained AUR package sources.

Packages:

- `librefang-bin`: installs the GitHub Release Linux binary tarball.
  Provides the CLI, daemon, HTTP API, and browser dashboard on port 4545.
- `librefang-desktop-bin`: installs the GitHub Release Tauri desktop bundle.
  Provides a native desktop launcher through `/usr/share/applications/LibreFang.desktop`.
- `librefang-docker`: installs a Docker-backed systemd/helper runner pinned to the same release tag.

No separate `librefang-web` package is needed.
The dashboard assets are already built into the release binaries, desktop bundle, and Docker image.

No separate first-party channel sidecar package is needed for normal users.
The daemon embeds the Python `librefang.sidecar` SDK and extracts it on demand when `python` is available.

Arch package versions cannot contain `-`.
Encode upstream prerelease tags by replacing the first `-` with `_`.

Example:

```text
v2026.6.24-beta.23 -> pkgver=2026.6.24_beta.23
```

Before publishing to AUR, run from each package directory:

```bash
makepkg -g
makepkg --printsrcinfo > .SRCINFO
makepkg -f
pacman -Qp ./*.pkg.tar.zst
pacman -Qlp ./*.pkg.tar.zst
```

For the binary package, also verify the staged binary:

```bash
pkg/librefang-bin/usr/bin/librefang --version
```

For the Docker package, verify the pinned image:

```bash
docker pull ghcr.io/librefang/librefang:<upstream-version>
docker run --rm --network none ghcr.io/librefang/librefang:<upstream-version> librefang --version
```

For the desktop package, verify the launcher:

```bash
pacman -Qlp ./librefang-desktop-bin-*.pkg.tar.zst
pkg/librefang-desktop-bin/usr/bin/librefang-desktop --help
sed -n '1,120p' pkg/librefang-desktop-bin/usr/share/applications/LibreFang.desktop
```

Only commit the AUR source files.
Do not commit downloaded sources, `src/`, `pkg/`, or `*.pkg.tar.*` outputs.

## Automated publishing on release

`release.yml` publishes these packages to AUR on every tag via three jobs (`sync_aur_bin`, `sync_aur_desktop`, `sync_aur_docker`).
Each job runs `publish-to-aur.sh` inside an `archlinux:base-devel` container, which takes the committed files here as the source of truth and derives only the per-release values: it bumps `pkgver` (encoding the tag's first `-` as `_`), resets `pkgrel` to `1`, sets `_desktop_ver` from the actual `LibreFang_<bundle-ver>_amd64.deb` asset name for the desktop package, re-pins the `ghcr.io/librefang/librefang:<version>` tag inside the Docker helper and env, then regenerates `sha256sums` (`updpkgsums`) and `.SRCINFO` (`makepkg --printsrcinfo`) before pushing to `ssh://aur@aur.archlinux.org/<package>.git`.

The committed `pkgver` / `sha256sums` / `.SRCINFO` here are not bumped per release; they are a working baseline for local `makepkg` runs.
The release-correct values live in the AUR repositories, regenerated on each tag.

The automation is inert until a maintainer configures the secrets — when `AUR_SSH_PRIVATE_KEY` is absent the jobs no-op with a notice.

### One-time maintainer bootstrap

1. Create the three AUR repositories under the AUR account that will own them — push an initial commit (or let the first release populate them):

   - `ssh://aur@aur.archlinux.org/librefang-bin.git`
   - `ssh://aur@aur.archlinux.org/librefang-desktop-bin.git`
   - `ssh://aur@aur.archlinux.org/librefang-docker.git`

2. Generate a dedicated CI keypair, register the public half on that AUR account, and add the secrets (`AUR_SSH_PRIVATE_KEY` and the optional `AUR_KNOWN_HOSTS` / `AUR_GIT_NAME` / `AUR_GIT_EMAIL`).
   See `.github/SECRETS.md` for the exact commands and rotation policy.

3. Validate end-to-end with `workflow_dispatch` on `release.yml` (`channel=current`, `tag=<an existing release tag>`) before the next real release.
