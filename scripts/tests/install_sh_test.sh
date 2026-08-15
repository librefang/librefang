#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
INSTALLER_PATH="$ROOT_DIR/web/public/install.sh"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

pass() {
    echo "PASS: $*"
}

# Point TMPDIR at a root of our own before the first mktemp call, so every `mktemp -d` in the suite lands underneath it and one trap reclaims all of them.
# Without this the suite leaves a temp tree behind per fixture directory on both the success path and the `fail` path, since `fail` exits straight out.
TEST_TMP_ROOT=$(mktemp -d)
TMPDIR="$TEST_TMP_ROOT"
export TMPDIR
trap 'rm -rf "$TEST_TMP_ROOT"' EXIT HUP INT TERM

TMP_HOME=$(mktemp -d)
HOME="$TMP_HOME" LIBREFANG_INSTALLER_SOURCE_ONLY=1 . "$INSTALLER_PATH"

# shell_rc_from_shell mappings
[ "$(shell_rc_from_shell zsh)" = "$TMP_HOME/.zshrc" ] || fail "zsh rc mapping"
[ "$(shell_rc_from_shell /bin/bash)" = "$TMP_HOME/.bashrc" ] || fail "bash rc mapping"
[ "$(shell_rc_from_shell fish)" = "$TMP_HOME/.config/fish/config.fish" ] || fail "fish rc mapping"
pass "shell_rc_from_shell mappings"

# choose_shell_rc: $SHELL fallback when detect_user_shell came back empty.
# Real-world hit: curl|sh pipelines where `ps -p $PPID -o comm=` returns
# something unexpected and USER_SHELL ends up blank.
mkdir -p "$TMP_HOME/.config/fish"
: > "$TMP_HOME/.config/fish/config.fish"
: > "$TMP_HOME/.zshrc"
: > "$TMP_HOME/.bashrc"
[ "$(SHELL=/usr/bin/zsh choose_shell_rc "")" = "$TMP_HOME/.zshrc" ] \
    || fail "empty arg + SHELL=zsh should pick .zshrc"
[ "$(SHELL=/bin/bash choose_shell_rc "")" = "$TMP_HOME/.bashrc" ] \
    || fail "empty arg + SHELL=bash should pick .bashrc"
[ "$(SHELL=/usr/bin/fish choose_shell_rc "")" = "$TMP_HOME/.config/fish/config.fish" ] \
    || fail "empty arg + SHELL=fish should pick fish config"
pass "choose_shell_rc uses \$SHELL when detect returned empty"

# File-existence fallback: when both the arg and $SHELL are unusable, prefer
# .zshrc > .bashrc > fish. Old order (bashrc first) silently wrote PATH into
# .bashrc for zsh users whose shell detection had failed upstream — zsh then
# can't see librefang in new shells.
[ "$(SHELL= choose_shell_rc "")" = "$TMP_HOME/.zshrc" ] \
    || fail "file fallback should prefer .zshrc over .bashrc"
rm -f "$TMP_HOME/.zshrc"
[ "$(SHELL= choose_shell_rc "")" = "$TMP_HOME/.bashrc" ] \
    || fail "file fallback should pick .bashrc when .zshrc missing"
rm -f "$TMP_HOME/.bashrc"
[ "$(SHELL= choose_shell_rc "")" = "$TMP_HOME/.config/fish/config.fish" ] \
    || fail "file fallback should pick fish config last"
pass "choose_shell_rc file-existence fallback order"

# The "already installed" check must match the install path, not any line
# mentioning the word "librefang". Prior `grep -q "librefang"` was too loose:
# a user named `librefang` (HOME=/home/librefang) caused any .zshrc line
# containing that path fragment — oh-my-zsh cache vars, plugin paths, a
# comment — to silently suppress the PATH append, leaving the shell with no
# way to find the binary.
: > "$TMP_HOME/.zshrc"
: > "$TMP_HOME/.bashrc"
echo 'ZSH_CACHE_DIR="/home/librefang/.cache/oh-my-zsh"' >> "$TMP_HOME/.zshrc"
echo '# user note: librefang install coming soon' >> "$TMP_HOME/.zshrc"
grep -qE "\.librefang/bin" "$TMP_HOME/.zshrc" \
    && fail "rc with only librefang-in-path words should not match \.librefang/bin"

echo 'export PATH="/home/alice/.librefang/bin:$PATH"' >> "$TMP_HOME/.zshrc"
grep -qE "\.librefang/bin" "$TMP_HOME/.zshrc" \
    || fail "rc with real librefang/bin PATH export should match"
pass "already-installed check uses precise \.librefang/bin pattern"

# auto-start flag parser
for truthy in 1 true TRUE yes YES on ON; do
    is_enabled "$truthy" || fail "is_enabled should accept $truthy"
done
for falsy in 0 false FALSE no NO off OFF ""; do
    if is_enabled "$falsy"; then
        fail "is_enabled should reject $falsy"
    fi
done
pass "LIBREFANG_AUTO_START flag parser"

# Parent-shell detection regression test with mocked ps. Responses are keyed
# by the requested field and PID so an extra ps call cannot silently change
# what the fixture means.
FAKE_BIN=$(mktemp -d)
cat > "$FAKE_BIN/ps" <<'PS_EOF'
#!/bin/sh
PID=""
PREVIOUS=""
for ARG in "$@"; do
  if [ "$PREVIOUS" = "-p" ]; then PID="$ARG"; fi
  PREVIOUS="$ARG"
done

case "$*" in
  *" -o ppid="*) echo "222"; exit 0 ;;
  *" -o comm="*)
    if [ "$PID" = "222" ]; then echo "zsh"; else echo "sh"; fi
    exit 0
    ;;
esac
exit 1
PS_EOF
chmod +x "$FAKE_BIN/ps"

if ! DETECTED=$(HOME="$TMP_HOME" PATH="$FAKE_BIN:$PATH" SHELL=/bin/bash INSTALLER_PATH="$INSTALLER_PATH" LIBREFANG_INSTALLER_SOURCE_ONLY=1 sh -c '. "$INSTALLER_PATH"; detect_user_shell'); then
    fail "detect_user_shell subshell exited non-zero"
fi
[ "$DETECTED" = "zsh" ] || fail "detect_user_shell expected zsh, got: $DETECTED"
pass "detect_user_shell handles curl|sh parent shell"

# Exercise the production PATH decision rather than duplicating its case arm.
session_needs_path_refresh "/nonexistent/test/.librefang/bin" \
    || fail "missing install dir should require a PATH refresh"
FIRST_PATH_ENTRY=$(printf "%s" "$PATH" | cut -d: -f1)
if session_needs_path_refresh "$FIRST_PATH_ENTRY"; then
    fail "an existing PATH entry should not require a refresh"
fi

# INSTALL_DIR is configurable and may contain shell glob characters. The
# comparison must treat them literally rather than as a case pattern.
ORIGINAL_PATH=$PATH
PATH='/tmp/exact:/tmp/literal-star'
session_needs_path_refresh '/tmp/*' \
    || fail "a wildcard-like install dir absent from PATH should require a refresh"
if session_needs_path_refresh '/tmp/literal-star'; then
    fail "a literal install dir present in PATH should not require a refresh"
fi
PATH=$ORIGINAL_PATH
pass "SESSION_NEEDS_PATH_REFRESH detection"

# Exercise the production restart-shell choice.
[ "$(SHELL=/bin/bash restart_shell_for zsh)" = "/bin/bash" ] \
    || fail "restart_shell_for should prefer SHELL"
pass "RESTART_SHELL prefers \$SHELL"

[ "$(SHELL= restart_shell_for zsh)" = "zsh" ] \
    || fail "restart_shell_for should fall back to USER_SHELL"
pass "RESTART_SHELL falls back to USER_SHELL when SHELL is empty"

# --- detect_distro: /etc/os-release parsing ------------------------------
# /etc/os-release and /etc/NIXOS are absolute paths no PATH mock can reach, so the installer indirects both through globals this suite points at fixtures.
DISTRO_FIXTURES=$(mktemp -d)
NIXOS_MARKER_ABSENT="$DISTRO_FIXTURES/no-nixos-marker"
NIXOS_MARKER_PRESENT="$DISTRO_FIXTURES/NIXOS"
: > "$NIXOS_MARKER_PRESENT"

cat > "$DISTRO_FIXTURES/os-release-nixos" <<'NIXOS_RELEASE_EOF'
NAME=NixOS
ID=nixos
VERSION="25.05 (Warbler)"
NIXOS_RELEASE_EOF

# ID=Deepin is deliberately capitalized: os-release does not normalize case, and the obvious lowercasing reach (${ID,,}) is a bashism that dash parses and then fails on at runtime.
cat > "$DISTRO_FIXTURES/os-release-deepin" <<'DEEPIN_RELEASE_EOF'
PRETTY_NAME="Deepin 20.9"
NAME="Deepin"
ID=Deepin
ID_LIKE=debian
DEEPIN_RELEASE_EOF

cat > "$DISTRO_FIXTURES/os-release-ubuntu" <<'UBUNTU_RELEASE_EOF'
NAME="Ubuntu"
ID=ubuntu
ID_LIKE=debian
VERSION_ID="24.04"
UBUNTU_RELEASE_EOF

cat > "$DISTRO_FIXTURES/os-release-inherited-debian" <<'INHERITED_RELEASE_EOF'
NAME="Debian GNU/Linux"
ID=debian
INHERITED_RELEASE_EOF

# A constructed fixture with ID=deepin and no ID_LIKE at all.
# The `*" deepin "*` arm of is_debian_family is the only thing that keeps such a host in the family, and the fixture above cannot reach that arm: its ID_LIKE=debian makes the joined string match the earlier `*" debian "*` arm first.
cat > "$DISTRO_FIXTURES/os-release-deepin-bare" <<'DEEPIN_BARE_RELEASE_EOF'
NAME="Deepin"
ID=deepin
DEEPIN_BARE_RELEASE_EOF

# A quoted, multi-entry ID_LIKE is the shape Debian derivatives-of-derivatives ship.
# Both the quote stripping and the whole list have to survive, or a derivative whose own ID nobody enumerated stops matching the family.
cat > "$DISTRO_FIXTURES/os-release-derivative" <<'DERIVATIVE_RELEASE_EOF'
NAME="Some Derivative"
ID=somederivative
ID_LIKE="ubuntu debian"
DERIVATIVE_RELEASE_EOF

# os-release permits shell-style assignments, not whitespace around '=' or
# trailing shell comments. Invalid lookalikes must not shadow a later valid
# field, while blank lines and comment lines are harmless.
cat > "$DISTRO_FIXTURES/os-release-edge-syntax" <<'EDGE_RELEASE_EOF'

# ignored comment
ID = spoofed
ID=ubuntu
ID_LIKE=debian
EDGE_RELEASE_EOF

OS_RELEASE_FILE="$DISTRO_FIXTURES/os-release-nixos"
NIXOS_MARKER_FILE="$NIXOS_MARKER_ABSENT"
detect_distro
[ "$DISTRO" = "nixos" ] || fail "detect_distro should report nixos, got: $DISTRO"
[ -z "$DISTRO_LIKE" ] || fail "nixos os-release has no ID_LIKE, got: $DISTRO_LIKE"
if is_debian_family; then fail "nixos should not be in the debian family"; fi
pass "detect_distro reads ID=nixos"

OS_RELEASE_FILE="$DISTRO_FIXTURES/os-release-deepin"
NIXOS_MARKER_FILE="$NIXOS_MARKER_ABSENT"
detect_distro
[ "$DISTRO" = "deepin" ] || fail "detect_distro should lowercase ID=Deepin, got: $DISTRO"
[ "$DISTRO_LIKE" = "debian" ] || fail "detect_distro should read deepin ID_LIKE, got: $DISTRO_LIKE"
is_debian_family || fail "deepin should be in the debian family"
pass "detect_distro lowercases ID and reads ID_LIKE for deepin"

OS_RELEASE_FILE="$DISTRO_FIXTURES/os-release-deepin-bare"
NIXOS_MARKER_FILE="$NIXOS_MARKER_ABSENT"
detect_distro
[ "$DISTRO" = "deepin" ] || fail "detect_distro should report deepin, got: $DISTRO"
[ -z "$DISTRO_LIKE" ] || fail "this fixture declares no ID_LIKE, got: $DISTRO_LIKE"
is_debian_family || fail "deepin with no ID_LIKE should still be in the debian family"
pass "is_debian_family matches deepin by its own ID when ID_LIKE is absent"

OS_RELEASE_FILE="$DISTRO_FIXTURES/os-release-ubuntu"
NIXOS_MARKER_FILE="$NIXOS_MARKER_ABSENT"
detect_distro
[ "$DISTRO" = "ubuntu" ] || fail "detect_distro should report ubuntu, got: $DISTRO"
[ "$DISTRO_LIKE" = "debian" ] || fail "detect_distro should read ubuntu ID_LIKE, got: $DISTRO_LIKE"
is_debian_family || fail "ubuntu should be in the debian family"
pass "detect_distro reads ID=ubuntu"

OS_RELEASE_FILE="$DISTRO_FIXTURES/os-release-derivative"
NIXOS_MARKER_FILE="$NIXOS_MARKER_ABSENT"
detect_distro
[ "$DISTRO" = "somederivative" ] || fail "detect_distro should keep an unknown ID verbatim, got: $DISTRO"
[ "$DISTRO_LIKE" = "ubuntu debian" ] \
    || fail "detect_distro should unquote a multi-entry ID_LIKE, got: $DISTRO_LIKE"
is_debian_family || fail "an ID_LIKE of \"ubuntu debian\" should be in the debian family"
pass "detect_distro matches the family through a quoted multi-entry ID_LIKE"

OS_RELEASE_FILE="$DISTRO_FIXTURES/os-release-edge-syntax"
NIXOS_MARKER_FILE="$NIXOS_MARKER_ABSENT"
detect_distro
[ "$DISTRO" = "ubuntu" ] || fail "invalid spaced assignment should not shadow ID, got: $DISTRO"
[ "$DISTRO_LIKE" = "debian" ] || fail "edge fixture should retain valid ID_LIKE, got: $DISTRO_LIKE"
pass "detect_distro ignores comments and invalid spaced assignments"

# /etc/NIXOS is authoritative: a container image that carried its base layer's ID=debian into a NixOS host must still be treated as NixOS, because the ELF interpreter the glibc build needs is missing either way.
OS_RELEASE_FILE="$DISTRO_FIXTURES/os-release-inherited-debian"
NIXOS_MARKER_FILE="$NIXOS_MARKER_PRESENT"
detect_distro
[ "$DISTRO" = "nixos" ] || fail "the NixOS marker should outrank an inherited ID=debian, got: $DISTRO"
pass "detect_distro treats the NixOS marker as authoritative"

# No os-release at all (macOS, minimal containers): degrade silently, never fail.
OS_RELEASE_FILE="$DISTRO_FIXTURES/os-release-does-not-exist"
NIXOS_MARKER_FILE="$NIXOS_MARKER_ABSENT"
# Assert the negative fixtures really are absent, or the "unknown" assertion below passes vacuously.
[ ! -e "$OS_RELEASE_FILE" ] || fail "the missing-os-release fixture must not exist: $OS_RELEASE_FILE"
[ ! -e "$NIXOS_MARKER_FILE" ] || fail "the absent-marker fixture must not exist: $NIXOS_MARKER_FILE"
detect_distro || fail "detect_distro must not fail when os-release is absent"
[ "$DISTRO" = "unknown" ] || fail "missing os-release should yield unknown, got: $DISTRO"
[ -z "$DISTRO_LIKE" ] || fail "missing os-release should leave DISTRO_LIKE empty, got: $DISTRO_LIKE"
if is_debian_family; then fail "unknown distro should not be in the debian family"; fi
pass "detect_distro degrades to unknown without /etc/os-release"

# --- effective_platform_fallback: NixOS cannot exec the glibc build ------
# NixOS has no /lib64/ld-linux-x86-64.so.2, so the gnu fallback must be ruled out before the download rather than after the post-install --version check rolls it back with a message that names no cause.
# The assertions read effective_platform_fallback rather than the PLATFORM_FALLBACK global, because that function is what both consumers consult — resolve_platform_for_tag and the download-retry branch — so a test that inspected the global instead would cover neither.
DISTRO="nixos"; DISTRO_LIKE=""
PLATFORM="x86_64-unknown-linux-musl"
PLATFORM_PRIMARY="$PLATFORM"
PLATFORM_FALLBACK="x86_64-unknown-linux-gnu"
[ -z "$(effective_platform_fallback)" ] \
    || fail "nixos should offer no fallback, got: $(effective_platform_fallback)"
apply_distro_platform_policy >/dev/null 2>&1 || fail "apply_distro_platform_policy should succeed on nixos"
[ "$PLATFORM" = "$PLATFORM_PRIMARY" ] || fail "nixos should keep the musl primary, got: $PLATFORM"
pass "effective_platform_fallback rules out the gnu fallback on NixOS"

DISTRO="ubuntu"; DISTRO_LIKE="debian"
[ "$(effective_platform_fallback)" = "x86_64-unknown-linux-gnu" ] \
    || fail "a glibc distro should keep the gnu fallback, got: $(effective_platform_fallback)"
apply_distro_platform_policy >/dev/null 2>&1 || fail "apply_distro_platform_policy should succeed on ubuntu"
pass "effective_platform_fallback keeps the gnu fallback off NixOS"

# apply_distro_platform_policy is now nothing but its explanation, so the explanation is what has to be asserted — discarding its output everywhere would leave the whole function uncovered.
# The generic rollback report ("The new binary failed to run") is precisely what this text exists to replace, so it has to name both NixOS and the missing interpreter.
DISTRO="nixos"; DISTRO_LIKE=""
PLATFORM="x86_64-unknown-linux-musl"
PLATFORM_PRIMARY="$PLATFORM"
PLATFORM_FALLBACK="x86_64-unknown-linux-gnu"
NIXOS_POLICY_MSG=$(apply_distro_platform_policy)
case "$NIXOS_POLICY_MSG" in
    *"NixOS detected"*) ;;
    *) fail "the policy step should say NixOS was detected, got: $NIXOS_POLICY_MSG" ;;
esac
case "$NIXOS_POLICY_MSG" in
    *"/lib64/ld-linux-x86-64.so.2"*) ;;
    *) fail "the policy step should name the missing ELF interpreter, got: $NIXOS_POLICY_MSG" ;;
esac
pass "apply_distro_platform_policy explains why NixOS gets no glibc fallback"

DISTRO="ubuntu"; DISTRO_LIKE="debian"
UBUNTU_POLICY_MSG=$(apply_distro_platform_policy)
[ -z "$UBUNTU_POLICY_MSG" ] \
    || fail "a glibc distro should get no policy message, got: $UBUNTU_POLICY_MSG"
pass "apply_distro_platform_policy stays silent off NixOS"

# A host whose detect_platform configures no fallback at all (darwin) must not have one invented for it.
DISTRO="unknown"; DISTRO_LIKE=""
PLATFORM="aarch64-apple-darwin"
PLATFORM_PRIMARY="$PLATFORM"
PLATFORM_FALLBACK=""
[ -z "$(effective_platform_fallback)" ] \
    || fail "a host with no fallback variant should stay empty, got: $(effective_platform_fallback)"
pass "effective_platform_fallback stays empty when the host configures no fallback"

# --- should_try_platform_fallback: the download-retry decision -----------
# The retry branch in install() reads nothing but this predicate, so its three states are asserted here: a fallback worth trying, a fallback PLATFORM already holds, and no usable fallback at all.
DISTRO="ubuntu"; DISTRO_LIKE="debian"
PLATFORM="x86_64-unknown-linux-musl"
PLATFORM_PRIMARY="$PLATFORM"
PLATFORM_FALLBACK="x86_64-unknown-linux-gnu"
should_try_platform_fallback \
    || fail "a fallback PLATFORM does not hold yet should be retried"

# resolve_platform_for_tag switches PLATFORM to the fallback during probing, so the download that just failed was already the fallback.
# Retrying then re-fetches the identical URL and blames a missing musl package that was never the target.
PLATFORM="$PLATFORM_FALLBACK"
if should_try_platform_fallback; then
    fail "the fallback must not be retried once PLATFORM already holds it"
fi

# No usable fallback: on NixOS because the gnu build cannot exec there, and on a host where detect_platform configured none in the first place.
PLATFORM="x86_64-unknown-linux-musl"
DISTRO="nixos"; DISTRO_LIKE=""
if should_try_platform_fallback; then
    fail "nixos must not retry the gnu fallback"
fi
DISTRO="ubuntu"; DISTRO_LIKE="debian"
PLATFORM_FALLBACK=""
if should_try_platform_fallback; then
    fail "a host with no fallback variant has nothing to retry"
fi
pass "should_try_platform_fallback covers every retry state"

# --- print_source_install_hint: NixOS gets the flake, not cargo ----------
# Assert only on substrings that contain no color variable: C_RED/C_YELLOW are set from `[ -t 1 ]` at source time, so they are empty under CI and populated when the suite runs in a terminal.
DISTRO="nixos"; DISTRO_LIKE=""
NIX_HINT=$(print_source_install_hint)
case "$NIX_HINT" in
    *"nix profile install github:librefang/librefang#librefang-cli"*) ;;
    *) fail "nixos hint should name the flake package, got: $NIX_HINT" ;;
esac
case "$NIX_HINT" in
    *"nix run github:librefang/librefang"*) ;;
    *) fail "nixos hint should offer nix run, got: $NIX_HINT" ;;
esac
case "$NIX_HINT" in
    *"services.librefang.enable = true;"*) ;;
    *) fail "nixos hint should point at the nixosModule for a persistent daemon, got: $NIX_HINT" ;;
esac
case "$NIX_HINT" in
    *"cargo install"*) fail "nixos hint should not send the user to cargo install: $NIX_HINT" ;;
esac
pass "print_source_install_hint points NixOS at the flake"

DISTRO="ubuntu"; DISTRO_LIKE="debian"
CARGO_HINT=$(print_source_install_hint)
case "$CARGO_HINT" in
    *"cargo install --git https://github.com/librefang/librefang librefang-cli"*) ;;
    *) fail "non-nixos hint should keep the cargo fallback, got: $CARGO_HINT" ;;
esac
pass "print_source_install_hint keeps cargo for non-NixOS hosts"

# --- print_debian_desktop_hint: probes, never asserts availability -------
DISTRO_OLD_PATH="$PATH"
FAKE_PROBE_BIN=$(mktemp -d)
cat > "$FAKE_PROBE_BIN/pkg-config" <<'PKGCONFIG_EOF'
#!/bin/sh
# --exists succeeds only for the module names listed in MOCK_PKGCONFIG_HAVE.
case "$*" in
    *--exists*)
        for want in ${MOCK_PKGCONFIG_HAVE:-}; do
            case " $* " in
                *" $want "*) exit 0 ;;
            esac
        done
        exit 1
        ;;
esac
exit 0
PKGCONFIG_EOF
chmod +x "$FAKE_PROBE_BIN/pkg-config"

cat > "$FAKE_PROBE_BIN/apt-cache" <<'APTCACHE_EOF'
#!/bin/sh
# `policy` prints nothing at all for a package the repositories do not carry;
# MOCK_APT_CANDIDATE names the single package with an installable candidate.
if [ "${1:-}" = "policy" ] && [ -n "${MOCK_APT_CANDIDATE:-}" ] && [ "${2:-}" = "$MOCK_APT_CANDIDATE" ]; then
    printf '%s:\n  Installed: (none)\n  Candidate: 2.44.0-2\n  Version table:\n' "$2"
fi
exit 0
APTCACHE_EOF
chmod +x "$FAKE_PROBE_BIN/apt-cache"

PATH="$FAKE_PROBE_BIN:$PATH"
MOCK_PKGCONFIG_HAVE=""
MOCK_APT_CANDIDATE=""
export MOCK_PKGCONFIG_HAVE MOCK_APT_CANDIDATE
SAVED_DISPLAY="${DISPLAY:-}"
SAVED_WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-}"
SAVED_XDG_SESSION_TYPE="${XDG_SESSION_TYPE:-}"
unset WAYLAND_DISPLAY XDG_SESSION_TYPE
DISPLAY=":0"

# Nothing answered: the hint must not name a webkit package, because claiming any particular distro ships libwebkit2gtk-4.1 would be a guess about a repository this script cannot see.
# PLATFORM is set explicitly rather than inherited from the block above, so this case does not silently depend on the previous group's leftovers.
DISTRO="deepin"; DISTRO_LIKE="debian"
PLATFORM="x86_64-unknown-linux-musl"
DEEPIN_HINT=$(print_debian_desktop_hint)
case "$DEEPIN_HINT" in
    *"apt-get install"*) fail "hint must prescribe no install command when no candidate was probed: $DEEPIN_HINT" ;;
esac
case "$DEEPIN_HINT" in
    *"could not be determined"*) ;;
    *) fail "hint should say the webkit series is undetermined, got: $DEEPIN_HINT" ;;
esac
case "$DEEPIN_HINT" in
    *"static musl build"*) ;;
    *) fail "deepin hint should say the CLI is the static musl build, got: $DEEPIN_HINT" ;;
esac
pass "print_debian_desktop_hint names no package when the probes find none"

# deepin does NOT suppress the gnu fallback, so a release with no musl package leaves PLATFORM on the glibc build.
# Claiming musl there would tell a user on an older release the opposite of what actually matters to them, which is precisely whether the host's glibc is new enough for the binary that got installed.
DISTRO="deepin"; DISTRO_LIKE="debian"
PLATFORM="x86_64-unknown-linux-gnu"
DEEPIN_GNU_HINT=$(print_debian_desktop_hint)
case "$DEEPIN_GNU_HINT" in
    *"static musl build"*) fail "hint must not claim musl when the gnu build was installed: $DEEPIN_GNU_HINT" ;;
esac
case "$DEEPIN_GNU_HINT" in
    *"does depend on the host's glibc"*) ;;
    *) fail "hint should warn that the gnu build depends on the host glibc, got: $DEEPIN_GNU_HINT" ;;
esac
PLATFORM="x86_64-unknown-linux-musl"
pass "print_debian_desktop_hint reads the installed variant off PLATFORM"

# apt offers only the 4.0 series: name exactly that, never the 4.1 package.
MOCK_PKGCONFIG_HAVE=""
MOCK_APT_CANDIDATE="libwebkit2gtk-4.0-dev"
PROBED_HINT=$(print_debian_desktop_hint)
case "$PROBED_HINT" in
    *"sudo apt-get install -y libwebkit2gtk-4.0-dev"*) ;;
    *) fail "hint should name the candidate the repositories offer, got: $PROBED_HINT" ;;
esac
case "$PROBED_HINT" in
    *"libwebkit2gtk-4.1-dev"*) fail "hint must not name a 4.1 package the repositories do not offer: $PROBED_HINT" ;;
esac
pass "print_debian_desktop_hint names only the probed apt candidate"

# pkg-config already reports 4.1: report the finding and prescribe nothing.
MOCK_PKGCONFIG_HAVE="webkit2gtk-4.1"
MOCK_APT_CANDIDATE=""
SATISFIED_HINT=$(print_debian_desktop_hint)
case "$SATISFIED_HINT" in
    *"pkg-config reports webkit2gtk-4.1 here"*) ;;
    *) fail "hint should report a satisfied 4.1 probe, got: $SATISFIED_HINT" ;;
esac
case "$SATISFIED_HINT" in
    *"apt-get install"*) fail "hint should prescribe nothing when 4.1 is already present: $SATISFIED_HINT" ;;
esac
pass "print_debian_desktop_hint reports a satisfied webkit2gtk-4.1 probe"

# A headless server has no graphical session, so the desktop hint stays silent.
MOCK_PKGCONFIG_HAVE=""
MOCK_APT_CANDIDATE=""
unset DISPLAY
HEADLESS_HINT=$(print_debian_desktop_hint)
[ -z "$HEADLESS_HINT" ] || fail "headless debian host should get no desktop hint, got: $HEADLESS_HINT"
pass "print_debian_desktop_hint stays silent without a graphical session"

# Off the Debian family the apt hint never applies, graphical session or not.
DISPLAY=":0"
DISTRO="nixos"; DISTRO_LIKE=""
NON_DEBIAN_HINT=$(print_debian_desktop_hint)
[ -z "$NON_DEBIAN_HINT" ] || fail "non-debian host should get no apt hint, got: $NON_DEBIAN_HINT"
pass "print_debian_desktop_hint stays silent off the Debian family"

unset DISPLAY
if [ -n "$SAVED_DISPLAY" ]; then DISPLAY="$SAVED_DISPLAY"; fi
if [ -n "$SAVED_WAYLAND_DISPLAY" ]; then WAYLAND_DISPLAY="$SAVED_WAYLAND_DISPLAY"; fi
if [ -n "$SAVED_XDG_SESSION_TYPE" ]; then XDG_SESSION_TYPE="$SAVED_XDG_SESSION_TYPE"; fi
PATH="$DISTRO_OLD_PATH"

# --- resolve_installable_version: asset-aware fallback --------------------
FAKE_CURL_BIN=$(mktemp -d)
cat > "$FAKE_CURL_BIN/curl" <<'CURL_EOF'
#!/bin/sh
# Mock curl for resolution tests. Driven by env:
#   MOCK_TAGS         newline-separated tags, newest first (release list)
#   MOCK_GOOD_TAGS    space-separated tags that have downloadable assets
#   MOCK_BAD_PLATFORM platform substring whose asset always 404s (optional)
for arg in "$@"; do
    case "$arg" in
        *"/releases?per_page="*)
            printf '%s\n' "${MOCK_TAGS:-}" | while IFS= read -r t; do
                [ -n "$t" ] && printf '    "tag_name": "%s",\n' "$t"
            done
            exit 0
            ;;
        *"/releases/download/"*)
            _t="${arg#*/releases/download/}"
            _t="${_t%%/*}"
            # The tarball probe must use a 1-byte range request; fail loudly if a
            # regression drops it (which would start pulling full archives). The
            # checksum probe (.sha256) is exempt — it is fetched in full.
            case "$arg" in
                *.tar.gz)
                    case " $* " in
                        *" -r 0-0 "*) ;;
                        *) echo "mock curl: tarball probe missing -r 0-0" >&2; exit 99 ;;
                    esac
                    ;;
            esac
            case " ${MOCK_GOOD_TAGS:-} " in
                *" $_t "*) ;;
                *) exit 22 ;;
            esac
            if [ -n "${MOCK_BAD_PLATFORM:-}" ]; then
                case "$arg" in
                    *"$MOCK_BAD_PLATFORM"*) exit 22 ;;
                esac
            fi
            exit 0
            ;;
    esac
done
exit 0
CURL_EOF
chmod +x "$FAKE_CURL_BIN/curl"

OLD_PATH="$PATH"
PATH="$FAKE_CURL_BIN:$PATH"
PLATFORM_PRIMARY="x86_64-unknown-linux-musl"
PLATFORM_FALLBACK="x86_64-unknown-linux-gnu"
# resolve_platform_for_tag consults effective_platform_fallback, which reads DISTRO, so the distro is set explicitly here instead of inheriting whatever the preceding block happened to leave behind.
DISTRO="ubuntu"; DISTRO_LIKE="debian"
MOCK_TAGS=$(printf '%s\n' "v3-stuck" "v2-good" "v1-good")
export MOCK_TAGS MOCK_GOOD_TAGS MOCK_BAD_PLATFORM
unset LIBREFANG_VERSION LIBREFANG_PREFERRED_VERSION

# Newest (v3-stuck) ships no assets -> fall back to v2-good.
MOCK_GOOD_TAGS="v2-good v1-good"; MOCK_BAD_PLATFORM=""
PLATFORM="$PLATFORM_PRIMARY"; VERSION=""
resolve_installable_version >/dev/null 2>&1 || fail "resolve should succeed when an older release is installable"
[ "$VERSION" = "v2-good" ] || fail "resolve should skip stuck newest, got: $VERSION"
pass "resolve_installable_version skips a stuck newest release"

# Platform fallback within a release: primary (musl) missing, fallback (gnu) ok.
MOCK_GOOD_TAGS="v2-good v1-good"; MOCK_BAD_PLATFORM="$PLATFORM_PRIMARY"
PLATFORM="$PLATFORM_PRIMARY"; VERSION=""
resolve_installable_version >/dev/null 2>&1 || fail "resolve should fall back to the gnu platform"
[ "$VERSION" = "v2-good" ] || fail "resolve version with platform fallback, got: $VERSION"
[ "$PLATFORM" = "$PLATFORM_FALLBACK" ] || fail "resolve should select the gnu platform, got: $PLATFORM"
pass "resolve_installable_version falls back across platform variants"

# Same release, but on NixOS: resolution must fail outright rather than select the gnu package, since installing a binary with no ELF interpreter is what produced the generic "The new binary failed to run" report on that distro.
# Suppression is driven through DISTRO, the way the installer really does it — blanking PLATFORM_FALLBACK by hand would assert a state no code path produces.
MOCK_GOOD_TAGS="v2-good v1-good"; MOCK_BAD_PLATFORM="$PLATFORM_PRIMARY"
PLATFORM="$PLATFORM_PRIMARY"; VERSION=""
DISTRO="nixos"; DISTRO_LIKE=""
if resolve_installable_version >/dev/null 2>&1; then
    fail "NixOS must not resolve the gnu platform, got: $PLATFORM"
fi
DISTRO="ubuntu"; DISTRO_LIKE="debian"
pass "resolve_installable_version cannot select gnu on NixOS"

# Explicit LIBREFANG_VERSION is a hard pin honored verbatim (no asset probe).
MOCK_GOOD_TAGS=""; MOCK_BAD_PLATFORM=""
export LIBREFANG_VERSION="v9-pinned"; VERSION=""; PLATFORM="$PLATFORM_PRIMARY"
resolve_installable_version >/dev/null 2>&1 || fail "hard pin should always resolve"
[ "$VERSION" = "v9-pinned" ] || fail "hard pin should set VERSION verbatim, got: $VERSION"
unset LIBREFANG_VERSION
pass "resolve_installable_version honors an explicit hard pin"

# LIBREFANG_PREFERRED_VERSION is a soft hint: used when its package exists, falls back when stuck.
MOCK_GOOD_TAGS="v2-good v1-good"; MOCK_BAD_PLATFORM=""
export LIBREFANG_PREFERRED_VERSION="v2-good"; VERSION=""; PLATFORM="$PLATFORM_PRIMARY"
resolve_installable_version >/dev/null 2>&1 || fail "preferred installable should resolve"
[ "$VERSION" = "v2-good" ] || fail "preferred installable should be used, got: $VERSION"
export LIBREFANG_PREFERRED_VERSION="v3-stuck"; VERSION=""; PLATFORM="$PLATFORM_PRIMARY"
resolve_installable_version >/dev/null 2>&1 || fail "stuck preferred should fall back"
[ "$VERSION" = "v2-good" ] || fail "stuck preferred should fall back to v2-good, got: $VERSION"
unset LIBREFANG_PREFERRED_VERSION
pass "resolve_installable_version treats preferred version as a soft hint"

# No installable release at all -> non-zero so install() can error out.
MOCK_GOOD_TAGS=""; MOCK_BAD_PLATFORM=""
PLATFORM="$PLATFORM_PRIMARY"; VERSION=""
if resolve_installable_version >/dev/null 2>&1; then
    fail "resolve should fail when no release ships a package"
fi
pass "resolve_installable_version fails when nothing is installable"

PATH="$OLD_PATH"

# --- ordering: the NixOS rule must not depend on call order ---------------
# The suppression was once a mutation of PLATFORM_FALLBACK that detect_platform re-set, so running the policy step before detect_platform silently restored the gnu fallback and NixOS was offered a binary it cannot exec.
# Nothing caught that, because every assertion pre-seeded PLATFORM_FALLBACK instead of letting detect_platform set it.
# These cases run the real sequence in both orders and assert the outcome is the same, which is what makes the ordering non-load-bearing rather than merely correct today.
# uname is mocked because detect_platform is under test here and the suite also runs on hosts whose `uname -s` is not Linux — the only OS that has a fallback at all.
FAKE_UNAME_BIN=$(mktemp -d)
cat > "$FAKE_UNAME_BIN/uname" <<'UNAME_EOF'
#!/bin/sh
case "${1:-}" in
    -m) echo "x86_64" ;;
    *) echo "Linux" ;;
esac
UNAME_EOF
chmod +x "$FAKE_UNAME_BIN/uname"

ORDER_OLD_PATH="$PATH"
PATH="$FAKE_UNAME_BIN:$FAKE_CURL_BIN:$PATH"
OS_RELEASE_FILE="$DISTRO_FIXTURES/os-release-nixos"
NIXOS_MARKER_FILE="$NIXOS_MARKER_ABSENT"

DISTRO=""; DISTRO_LIKE=""; PLATFORM=""; PLATFORM_PRIMARY=""; PLATFORM_FALLBACK=""
detect_distro
detect_platform
apply_distro_platform_policy >/dev/null 2>&1
# Assert the mock really drove detect_platform, or the comparison below is vacuous.
[ "$PLATFORM" = "x86_64-unknown-linux-musl" ] \
    || fail "the mocked uname should yield the linux musl primary, got: $PLATFORM"
[ "$PLATFORM_FALLBACK" = "x86_64-unknown-linux-gnu" ] \
    || fail "detect_platform should still record the raw gnu fallback, got: $PLATFORM_FALLBACK"
ORDER_A_FALLBACK=$(effective_platform_fallback)

DISTRO=""; DISTRO_LIKE=""; PLATFORM=""; PLATFORM_PRIMARY=""; PLATFORM_FALLBACK=""
detect_distro
apply_distro_platform_policy >/dev/null 2>&1
detect_platform
ORDER_B_FALLBACK=$(effective_platform_fallback)

[ -z "$ORDER_A_FALLBACK" ] \
    || fail "NixOS should offer no fallback in the shipped order, got: $ORDER_A_FALLBACK"
[ "$ORDER_A_FALLBACK" = "$ORDER_B_FALLBACK" ] \
    || fail "the fallback decision changed with call order: [$ORDER_A_FALLBACK] vs [$ORDER_B_FALLBACK]"
pass "the NixOS fallback decision is identical in both call orders"

# The consequence that actually matters: with the release shipping no musl package, resolution must fail in either order instead of selecting the gnu build.
MOCK_GOOD_TAGS="v2-good v1-good"; MOCK_BAD_PLATFORM="x86_64-unknown-linux-musl"
for order in shipped hoisted; do
    DISTRO=""; DISTRO_LIKE=""; PLATFORM=""; PLATFORM_PRIMARY=""; PLATFORM_FALLBACK=""; VERSION=""
    detect_distro
    if [ "$order" = "shipped" ]; then
        detect_platform
        apply_distro_platform_policy >/dev/null 2>&1
    else
        apply_distro_platform_policy >/dev/null 2>&1
        detect_platform
    fi
    if resolve_installable_version >/dev/null 2>&1; then
        fail "$order order resolved a platform on NixOS with no musl package: $PLATFORM"
    fi
done
pass "resolve_installable_version refuses the gnu build on NixOS in either call order"

PATH="$ORDER_OLD_PATH"

# --- install_binary_with_rollback: atomic replace + rollback -------------
RB_DIR=$(mktemp -d)
RB_DEST="$RB_DIR/librefang"
cat > "$RB_DEST" <<'OLD_EOF'
#!/bin/sh
[ "$1" = "--version" ] && echo "old 1.0"
OLD_EOF
chmod +x "$RB_DEST"

RB_GOOD="$RB_DIR/new-good"
cat > "$RB_GOOD" <<'GOOD_EOF'
#!/bin/sh
[ "$1" = "--version" ] && echo "new 2.0"
GOOD_EOF
chmod +x "$RB_GOOD"

install_binary_with_rollback "$RB_GOOD" "$RB_DEST" >/dev/null 2>&1 \
    || fail "install_binary_with_rollback should succeed for a working binary"
[ "$("$RB_DEST" --version)" = "new 2.0" ] || fail "working upgrade should install the new binary"
[ ! -e "$RB_DEST.bak" ] || fail "backup should be removed after a successful upgrade"
pass "install_binary_with_rollback installs a working new binary"

RB_BAD="$RB_DIR/new-bad"
cat > "$RB_BAD" <<'BAD_EOF'
#!/bin/sh
exit 1
BAD_EOF
chmod +x "$RB_BAD"

if install_binary_with_rollback "$RB_BAD" "$RB_DEST" >/dev/null 2>&1; then
    fail "install_binary_with_rollback should fail for a broken binary"
fi
[ "$("$RB_DEST" --version)" = "new 2.0" ] || fail "broken upgrade should roll back to the previous binary"
[ ! -e "$RB_DEST.bak" ] || fail "backup should be cleaned up after a rollback"
pass "install_binary_with_rollback rolls back a broken new binary"

# Fresh install (no existing binary) with a broken new binary must NOT leave a
# non-runnable binary on PATH — there is nothing to roll back to.
RB_FRESH="$RB_DIR/fresh/librefang"
mkdir -p "$RB_DIR/fresh"
if install_binary_with_rollback "$RB_BAD" "$RB_FRESH" >/dev/null 2>&1; then
    fail "install_binary_with_rollback should fail for a broken fresh install"
fi
[ ! -e "$RB_FRESH" ] || fail "broken fresh install should not leave a binary behind"
[ ! -e "$RB_FRESH.bak" ] || fail "broken fresh install should not leave a backup behind"
pass "install_binary_with_rollback removes a broken fresh install"

echo "All install.sh tests passed."
