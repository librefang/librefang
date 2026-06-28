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
