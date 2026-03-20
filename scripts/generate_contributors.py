#!/usr/bin/env python3

"""Generate a contributors SVG with round avatars from the GitHub API."""

from __future__ import annotations

import base64
import json
import os
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass

API_ROOT = "https://api.github.com"
PER_PAGE = 100

COLUMNS = 12
CELL = 54
AVATAR_R = 24
PAD_X = 3
PAD_Y = 3


@dataclass
class Contributor:
    login: str
    avatar_url: str
    html_url: str
    contributions: int


def github_request(url: str, retries: int = 3) -> list[dict]:
    headers = {
        "Accept": "application/vnd.github+json",
        "User-Agent": "librefang-contributors-generator",
    }
    token = os.environ.get("GITHUB_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    for attempt in range(retries):
        req = urllib.request.Request(url, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=30) as resp:
                return json.loads(resp.read())
        except urllib.error.HTTPError as e:
            if e.code == 403 and attempt < retries - 1:
                wait = 2 ** attempt * 5
                print(f"Rate limited, retrying in {wait}s...")
                time.sleep(wait)
                continue
            raise


def fetch_contributors(repo: str) -> list[Contributor]:
    contributors: list[Contributor] = []
    page = 1
    while True:
        url = f"{API_ROOT}/repos/{repo}/contributors?per_page={PER_PAGE}&page={page}"
        data = github_request(url)
        if not data:
            break
        for c in data:
            if c.get("type") != "User":
                continue
            contributors.append(
                Contributor(
                    login=c["login"],
                    avatar_url=c["avatar_url"],
                    html_url=c["html_url"],
                    contributions=c["contributions"],
                )
            )
        if len(data) < PER_PAGE:
            break
        page += 1
    return contributors


def fetch_avatar_base64(avatar_url: str, size: int = 96) -> str:
    url = f"{avatar_url}&s={size}"
    req = urllib.request.Request(url, headers={"User-Agent": "librefang-contributors-generator"})
    with urllib.request.urlopen(req, timeout=30) as resp:
        data = resp.read()
        content_type = resp.headers.get("Content-Type", "image/png")
    b64 = base64.b64encode(data).decode()
    return f"data:{content_type};base64,{b64}"


def render_svg(contributors: list[Contributor], max_count: int = 100) -> str:
    contributors = contributors[:max_count]
    count = len(contributors)
    cols = min(count, COLUMNS)
    rows = (count + cols - 1) // cols

    width = PAD_X * 2 + cols * CELL
    height = PAD_Y * 2 + rows * CELL

    parts: list[str] = []
    parts.append(
        f'<svg xmlns="http://www.w3.org/2000/svg" '
        f'xmlns:xlink="http://www.w3.org/1999/xlink" '
        f'width="{width}" height="{height}" viewBox="0 0 {width} {height}">'
    )

    # Defs: clip circles
    parts.append("  <defs>")
    for i in range(count):
        parts.append(f'    <clipPath id="clip-{i}"><circle cx="0" cy="0" r="{AVATAR_R}"/></clipPath>')
    parts.append("  </defs>")

    for i, c in enumerate(contributors):
        col = i % cols
        row = i // cols
        cx = PAD_X + col * CELL + CELL // 2
        cy = PAD_Y + row * CELL + CELL // 2

        avatar_data = fetch_avatar_base64(c.avatar_url)

        parts.append(f'  <a xlink:href="{c.html_url}" target="_blank">')
        parts.append(f'    <g transform="translate({cx},{cy})">')
        parts.append(
            f'      <image x="-{AVATAR_R}" y="-{AVATAR_R}" '
            f'width="{AVATAR_R * 2}" height="{AVATAR_R * 2}" '
            f'clip-path="url(#clip-{i})" '
            f'xlink:href="{avatar_data}"/>'
        )
        parts.append(
            f'      <circle cx="0" cy="0" r="{AVATAR_R}" '
            f'fill="none" stroke="#e1e4e8" stroke-width="1"/>'
        )
        parts.append("    </g>")
        parts.append("  </a>")

    parts.append("</svg>")
    return "\n".join(parts)


def main() -> None:
    if len(sys.argv) < 3:
        print(f"Usage: {sys.argv[0]} owner/repo output.svg", file=sys.stderr)
        sys.exit(1)

    repo = sys.argv[1]
    output = sys.argv[2]

    print(f"Fetching contributors for {repo}...")
    contributors = fetch_contributors(repo)
    print(f"Found {len(contributors)} contributors")

    for c in contributors:
        print(f"  {c.login}: {c.contributions} contributions")

    print("Generating SVG (downloading avatars)...")
    svg = render_svg(contributors)

    with open(output, "w") as f:
        f.write(svg)
    print(f"Wrote {output}")


if __name__ == "__main__":
    main()
