#!/usr/bin/env python3

from __future__ import annotations

import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from collections import Counter
from dataclasses import dataclass
from datetime import UTC, date, datetime, timedelta
from pathlib import Path


API_ROOT = "https://api.github.com"
PER_PAGE = 100


@dataclass
class Point:
    day: date
    stars: int


def github_request(url: str, retries: int = 3) -> list[dict]:
    headers = {
        "Accept": "application/vnd.github.star+json",
        "User-Agent": "librefang-star-history-generator",
        "X-GitHub-Api-Version": "2022-11-28",
    }
    token = os.getenv("GITHUB_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"

    for attempt in range(retries):
        request = urllib.request.Request(url, headers=headers)
        try:
            with urllib.request.urlopen(request) as response:
                return json.load(response)
        except urllib.error.HTTPError as e:
            if e.code == 403 and attempt < retries - 1:
                wait = 2 ** attempt * 5
                print(f"Rate limited, retrying in {wait}s...")
                time.sleep(wait)
                continue
            raise


def fetch_stargazers(repo: str) -> list[datetime]:
    starred = []
    page = 1

    while True:
        query = urllib.parse.urlencode({"per_page": PER_PAGE, "page": page})
        url = f"{API_ROOT}/repos/{repo}/stargazers?{query}"
        try:
            rows = github_request(url)
        except urllib.error.HTTPError as exc:
            body = exc.read().decode("utf-8", errors="replace")
            raise SystemExit(f"GitHub API request failed ({exc.code}): {body}") from exc

        if not rows:
            break

        for row in rows:
            starred_at = row.get("starred_at")
            if starred_at:
                starred.append(datetime.fromisoformat(starred_at.replace("Z", "+00:00")))

        if len(rows) < PER_PAGE:
            break
        page += 1

    starred.sort()
    return starred


def build_series(stars: list[datetime]) -> list[Point]:
    today = date.today()
    if not stars:
        return [Point(today, 0)]

    counts_by_day = Counter(star.date() for star in stars)
    first_day = min(counts_by_day)
    current_day = first_day - timedelta(days=1)
    end_day = max(max(counts_by_day), today)
    running_total = 0
    series: list[Point] = []

    while current_day <= end_day:
        running_total += counts_by_day.get(current_day, 0)
        series.append(Point(current_day, running_total))
        current_day += timedelta(days=1)

    return series


def svg_escape(text: str) -> str:
    return (
        text.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace('"', "&quot;")
    )


def render_svg(series: list[Point], repo: str) -> str:
    width = 800
    height = 320
    left = 68
    plot_left = left + 16
    right = 28
    top = 104
    bottom = 46
    chart_width = width - plot_left - right
    chart_height = height - top - bottom

    max_stars = max(point.stars for point in series)
    start_day = series[0].day
    end_day = series[-1].day
    total_days = max((end_day - start_day).days, 1)

    def x_for(day: date) -> float:
        return plot_left + (((day - start_day).days / total_days) * chart_width)

    def y_for(stars: int) -> float:
        if max_stars == 0:
            return top + chart_height
        return top + chart_height - ((stars / max_stars) * chart_height)

    points = " ".join(f"{x_for(point.day):.2f},{y_for(point.stars):.2f}" for point in series)

    label_days = [start_day, start_day + timedelta(days=total_days // 2), end_day]
    label_days = list(dict.fromkeys(label_days))
    y_labels = sorted({0, max_stars // 2 if max_stars else 0, max_stars})

    repo_label = svg_escape(repo)
    updated_label = svg_escape(datetime.now(UTC).strftime("%Y-%m-%d %H:%M UTC"))

    x_axis_labels = "\n".join(
        f'<text x="{x_for(day):.2f}" y="{height - 14}" class="axis-label" text-anchor="{"start" if index == 0 else "end" if index == len(label_days) - 1 else "middle"}">{day.isoformat()}</text>'
        for index, day in enumerate(label_days)
    )
    y_axis_labels = "\n".join(
        f'<text x="{left - 10}" y="{y_for(value) + 4:.2f}" class="axis-label" text-anchor="end">{value}</text>'
        for value in y_labels
    )
    horizontal_guides = "\n".join(
        f'<line x1="{left}" y1="{y_for(value):.2f}" x2="{width - right}" y2="{y_for(value):.2f}" class="grid" />'
        for value in y_labels
    )

    return f"""<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" role="img" aria-labelledby="title desc">
  <title id="title">Star History for {repo_label}</title>
  <desc id="desc">Cumulative GitHub stars over time for {repo_label}</desc>
  <style>
    .bg {{ fill: #0f172a; }}
    .panel {{ fill: #111827; }}
    .grid {{ stroke: #243244; stroke-width: 1; }}
    .axis {{ stroke: #475569; stroke-width: 1.25; }}
    .line {{ fill: none; stroke: #22c55e; stroke-width: 3; stroke-linecap: round; stroke-linejoin: round; }}
    .area {{ fill: url(#areaGradient); }}
    .title {{ font: 700 22px ui-sans-serif, system-ui, -apple-system, sans-serif; fill: #f8fafc; }}
    .subtitle {{ font: 400 12px ui-sans-serif, system-ui, -apple-system, sans-serif; fill: #94a3b8; }}
    .axis-label {{ font: 400 11px ui-sans-serif, system-ui, -apple-system, sans-serif; fill: #94a3b8; }}
    .value {{ font: 700 28px ui-sans-serif, system-ui, -apple-system, sans-serif; fill: #f8fafc; }}
  </style>
  <defs>
    <linearGradient id="areaGradient" x1="0" x2="0" y1="0" y2="1">
      <stop offset="0%" stop-color="#22c55e" stop-opacity="0.35" />
      <stop offset="100%" stop-color="#22c55e" stop-opacity="0.02" />
    </linearGradient>
  </defs>
  <rect width="{width}" height="{height}" rx="18" class="bg" />
  <rect x="12" y="12" width="{width - 24}" height="{height - 24}" rx="14" class="panel" />
  <text x="30" y="52" class="title">Star History</text>
  <text x="30" y="74" class="subtitle">{repo_label}</text>
  <text x="{width - 30}" y="52" class="value" text-anchor="end">{series[-1].stars}</text>
  <text x="{width - 30}" y="74" class="subtitle" text-anchor="end">Updated {updated_label}</text>
  {horizontal_guides}
  <line x1="{left}" y1="{top + chart_height}" x2="{width - right}" y2="{top + chart_height}" class="axis" />
  <line x1="{left}" y1="{top}" x2="{left}" y2="{top + chart_height}" class="axis" />
  <path d="M {plot_left},{top + chart_height} {' '.join(f'L {x_for(point.day):.2f},{y_for(point.stars):.2f}' for point in series)} L {width - right},{top + chart_height} Z" class="area" />
  <polyline points="{points}" class="line" />
  {x_axis_labels}
  {y_axis_labels}
</svg>
"""


def main() -> int:
    if len(sys.argv) != 3:
        print("Usage: generate_star_history.py <owner/repo> <output.svg>", file=sys.stderr)
        return 1

    repo = sys.argv[1]
    output = Path(sys.argv[2])
    output.parent.mkdir(parents=True, exist_ok=True)

    stars = fetch_stargazers(repo)
    series = build_series(stars)
    output.write_text(render_svg(series, repo), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
