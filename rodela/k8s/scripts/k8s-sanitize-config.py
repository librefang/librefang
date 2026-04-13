#!/usr/bin/env python3
"""
k8s-sanitize-config.py

Extracts sanitized config files from an RPi .tar.gz tarball and writes them
to an output directory. Secret fields (api_key, dashboard_pass) are blanked
in-place so the result is safe to commit to version control.

Usage:
    python3 k8s-sanitize-config.py <tarball.tar.gz> <output_dir>

What gets extracted:
  - config.toml         (api_key and dashboard_pass blanked)
  - aliases.toml        (no sensitive fields, copied as-is)
  - channels/*.toml     (api_key blanked if present)
  - providers/*.toml    (api_key blanked if present)
  - integrations/*.toml (api_key blanked if present)

Files that contain secrets but are NOT safe to commit (.env, secrets.env,
vault.enc) are intentionally excluded — use k8s-extract-secrets.py for those.
"""

import os
import re
import sys
import tarfile


# ---------------------------------------------------------------------------
# Sanitization helpers
# ---------------------------------------------------------------------------

def blank_secret_fields(content: str) -> str:
    """Blank api_key and dashboard_pass TOML string fields."""
    content = re.sub(
        r'^(api_key\s*=\s*).*$',
        r'\1""',
        content,
        flags=re.MULTILINE,
    )
    content = re.sub(
        r'^(dashboard_pass\s*=\s*).*$',
        r'\1""',
        content,
        flags=re.MULTILINE,
    )
    return content


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


def write_file(output_dir: str, rel_path: str, content: str) -> str:
    """Write content to <output_dir>/<rel_path>, creating parent dirs as needed."""
    dest = os.path.join(output_dir, rel_path)
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    with open(dest, 'w', encoding='utf-8') as fh:
        fh.write(content)
    return dest


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main() -> None:
    if len(sys.argv) < 3:
        print("Usage: k8s-sanitize-config.py <tarball.tar.gz> <output_dir>", file=sys.stderr)
        sys.exit(1)

    tarball_path = sys.argv[1]
    output_dir = sys.argv[2]
    os.makedirs(output_dir, exist_ok=True)

    written: list[str] = []

    with tarfile.open(tarball_path, 'r:gz') as tf:
        # --- config.toml (blank secrets) ---
        content = read_tar_member(tf, 'config.toml')
        if content:
            sanitized = blank_secret_fields(content)
            dest = write_file(output_dir, 'config.toml', sanitized)
            written.append(dest)
        else:
            print("WARNING: config.toml not found in tarball", file=sys.stderr)

        # --- aliases.toml (no sensitive fields) ---
        content = read_tar_member(tf, 'aliases.toml')
        if content:
            dest = write_file(output_dir, 'aliases.toml', content)
            written.append(dest)

        # --- Per-directory TOML files ---
        # channels/, providers/, integrations/ — blank api_key in provider files.
        sensitive_dirs = {'providers'}
        config_dirs = {'channels', 'providers', 'integrations'}

        all_members = tf.getmembers()
        for member in all_members:
            # Normalise path: strip leading './'
            norm = member.name.lstrip('./')
            if not norm:
                continue

            parts = norm.split('/')
            if len(parts) < 2:
                continue

            top_dir = parts[0]
            if top_dir not in config_dirs:
                continue

            if not norm.endswith('.toml'):
                continue

            f = tf.extractfile(member)
            if f is None:
                continue

            file_content = f.read().decode('utf-8', errors='replace')

            # Blank api_key in provider directories
            if top_dir in sensitive_dirs:
                file_content = re.sub(
                    r'^(api_key\s*=\s*).*$',
                    r'\1""',
                    file_content,
                    flags=re.MULTILINE,
                )

            dest = write_file(output_dir, norm, file_content)
            written.append(dest)

    # --- Summary ---
    print(f"Extracted {len(written)} files to {output_dir}")
    for path in sorted(written):
        print(f"  {path}")


if __name__ == '__main__':
    main()
