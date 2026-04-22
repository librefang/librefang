#!/usr/bin/env python3
"""
k8s-extract-secrets.py

Reads a .tar.gz tarball (path from sys.argv[1]) produced by k8s-migrate-from-rpi.sh,
extracts all secret values, and writes KEY=value lines to STDOUT ONLY.

The caller is responsible for redirecting stdout to secrets.env, e.g.:
    python3 k8s-extract-secrets.py /tmp/librefang-rpi-*.tar.gz > deploy/k8s/secrets.env

Sources (in priority order, later values overwrite earlier):
  1. config.toml  -> api_key       => LIBREFANG_API_KEY
                     dashboard_pass => LIBREFANG_DASHBOARD_PASS
  2. .env         -> passed through as-is
  3. secrets.env  -> passed through as-is

Output is sorted by key for deterministic diffs.
This script never writes to any file directly.
"""

import re
import sys
import tarfile


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def extract_toml_value(content: str, key: str) -> str:
    """Return the value of a TOML string field, or empty string if not found."""
    m = re.search(
        rf'^{re.escape(key)}\s*=\s*["\']([^"\']*)["\']',
        content,
        re.MULTILINE,
    )
    return m.group(1) if m else ""


def parse_env_lines(content: str) -> dict:
    """Parse KEY=value lines; skip comments and blanks; strip surrounding quotes."""
    result = {}
    for line in content.splitlines():
        line = line.strip()
        if not line or line.startswith('#'):
            continue
        if '=' in line:
            k, _, v = line.partition('=')
            result[k.strip()] = v.strip().strip('"\'')
    return result


def read_tar_member(tf: tarfile.TarFile, name: str) -> str | None:
    """
    Return the text content of a tarball member, or None if not found.
    Tries both the bare name and with a leading './' prefix.
    """
    for candidate in (name, f'./{name}'):
        try:
            member = tf.getmember(candidate)
            f = tf.extractfile(member)
            if f is not None:
                return f.read().decode('utf-8', errors='replace')
        except KeyError:
            continue
    return None


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main() -> None:
    if len(sys.argv) < 2:
        print("Usage: k8s-extract-secrets.py <tarball.tar.gz>", file=sys.stderr)
        sys.exit(1)

    tarball_path = sys.argv[1]
    secrets: dict[str, str] = {}

    with tarfile.open(tarball_path, 'r:gz') as tf:
        # --- config.toml ---
        config_content = read_tar_member(tf, 'config.toml')
        if config_content:
            api_key = extract_toml_value(config_content, 'api_key')
            dashboard_pass = extract_toml_value(config_content, 'dashboard_pass')
            if api_key:
                secrets['LIBREFANG_API_KEY'] = api_key
            if dashboard_pass:
                secrets['LIBREFANG_DASHBOARD_PASS'] = dashboard_pass
        else:
            print("WARNING: config.toml not found in tarball", file=sys.stderr)

        # --- .env ---
        env_content = read_tar_member(tf, '.env')
        if env_content:
            secrets.update(parse_env_lines(env_content))

        # --- secrets.env ---
        secrets_env_content = read_tar_member(tf, 'secrets.env')
        if secrets_env_content:
            secrets.update(parse_env_lines(secrets_env_content))

    # Write sorted KEY=value pairs to stdout
    for key in sorted(secrets.keys()):
        print(f"{key}={secrets[key]}")


if __name__ == '__main__':
    main()
