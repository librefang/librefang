#!/usr/bin/env python3
"""Publish idempotent notifications for a completed unified Release run."""

from __future__ import annotations

import json
import os
import re
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path


class NotifyError(RuntimeError):
    pass


def public_origin(url):
    parsed = urllib.parse.urlsplit(url)
    scheme = parsed.scheme or "https"
    host = parsed.hostname or "unknown-host"
    return f"{scheme}://{host}"


class HttpClient:
    def request(self, method, url, *, headers=None, payload=None):
        body = None if payload is None else json.dumps(payload).encode()
        request_headers = dict(headers or {})
        if payload is not None:
            request_headers.setdefault("Content-Type", "application/json")
        request = urllib.request.Request(url, data=body, headers=request_headers, method=method)
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                return response.status, response.read().decode()
        except urllib.error.HTTPError as exc:
            return exc.code, exc.read().decode(errors="replace")
        except (urllib.error.URLError, TimeoutError) as exc:
            reason = getattr(exc, "reason", str(exc))
            raise NotifyError(
                f"request to {public_origin(url)} failed: {reason}"
            ) from exc


class GitHub:
    def __init__(self, repository, token, http):
        self.repository = repository
        self.owner, self.name = repository.split("/", 1)
        self.http = http
        self.headers = {
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "X-GitHub-Api-Version": "2022-11-28",
        }

    def _json(self, method, path, payload=None):
        status, body = self.http.request(
            method,
            f"https://api.github.com/{path.lstrip('/')}",
            headers=self.headers,
            payload=payload,
        )
        if not 200 <= status < 300:
            raise NotifyError(f"GitHub API {path} returned HTTP {status}: {body[:500]}")
        try:
            return json.loads(body) if body else None
        except json.JSONDecodeError as exc:
            raise NotifyError(f"GitHub API {path} returned invalid JSON") from exc

    def run_jobs(self, run_id, page_size=100):
        jobs = []
        page = 1
        while True:
            data = self._json(
                "GET",
                f"repos/{self.repository}/actions/runs/{run_id}/jobs?per_page={page_size}&page={page}",
            )
            batch = data.get("jobs") if isinstance(data, dict) else None
            if not isinstance(batch, list):
                raise NotifyError("GitHub jobs response omitted jobs array")
            jobs.extend(batch)
            if len(batch) < page_size:
                return jobs
            page += 1

    def marker_done(self, sha, channel, page_size=100):
        context = f"release-notify/{channel}"
        page = 1
        while True:
            statuses = self._json(
                "GET",
                f"repos/{self.repository}/commits/{sha}/statuses"
                f"?per_page={page_size}&page={page}",
            )
            if not isinstance(statuses, list):
                raise NotifyError("GitHub statuses response was not an array")
            if any(
                status.get("context") == context and status.get("state") == "success"
                for status in statuses
            ):
                return True
            if len(statuses) < page_size:
                return False
            page += 1

    def mark_done(self, sha, channel, description):
        self._json(
            "POST",
            f"repos/{self.repository}/statuses/{sha}",
            {
                "state": "success",
                "context": f"release-notify/{channel}",
                "description": description[:140],
            },
        )

    def graphql(self, query, variables):
        data = self._json("POST", "graphql", {"query": query, "variables": variables})
        if not isinstance(data, dict):
            raise NotifyError("GitHub GraphQL returned a non-object response")
        if data.get("errors"):
            raise NotifyError(f"GitHub GraphQL failed: {json.dumps(data['errors'])[:500]}")
        return data.get("data", {})

    def discussion_target(self, title):
        query = """
        query($owner:String!,$name:String!,$after:String){
          repository(owner:$owner,name:$name){
            id
            discussionCategories(first:100){nodes{id name}}
            discussions(first:100,after:$after,orderBy:{field:CREATED_AT,direction:DESC}){
              nodes{title url}
              pageInfo{hasNextPage endCursor}
            }
          }
        }
        """
        after = None
        while True:
            repo = self.graphql(
                query, {"owner": self.owner, "name": self.name, "after": after}
            ).get("repository")
            if not repo:
                raise NotifyError("GitHub repository was not returned by GraphQL")
            category = next(
                (
                    node
                    for node in repo["discussionCategories"]["nodes"]
                    if node["name"] == "Announcements"
                ),
                None,
            )
            if not category:
                raise NotifyError("GitHub Discussion category 'Announcements' was not found")
            existing = next(
                (
                    node["url"]
                    for node in repo["discussions"]["nodes"]
                    if node["title"] == title
                ),
                None,
            )
            page_info = repo["discussions"]["pageInfo"]
            if existing or not page_info["hasNextPage"]:
                return repo["id"], category["id"], existing
            after = page_info["endCursor"]

    def create_discussion(self, repo_id, category_id, title, body):
        mutation = """
        mutation($repo:ID!,$category:ID!,$title:String!,$body:String!){
          createDiscussion(input:{repositoryId:$repo,categoryId:$category,title:$title,body:$body}){
            discussion{url}
          }
        }
        """
        data = self.graphql(
            mutation,
            {"repo": repo_id, "category": category_id, "title": title, "body": body},
        )
        url = data.get("createDiscussion", {}).get("discussion", {}).get("url")
        if not url:
            raise NotifyError("GitHub createDiscussion response omitted the discussion URL")
        return url


GROUPS = {
    "CLI": lambda name: name.startswith("CLI /")
    or name == "Sign Release Artifacts"
    or name == "Sync to Homebrew Tap"
    or name == "Publish AUR / librefang-bin"
    or name == "Publish pacman repo (Arch)",
    "Desktop": lambda name: name.startswith(("Desktop /", "Mobile /"))
    or name == "Sync Homebrew Cask"
    or name == "Publish AUR / librefang-desktop-bin",
    "Docker": lambda name: name.startswith(("Docker /", "Docker Scan /"))
    or name == "Docker Manifest"
    or name == "Publish AUR / librefang-docker",
    "Deploy": lambda name: name.startswith("Deploy to "),
    "SDK": lambda name: name.startswith("SDK /"),
}


def aggregate_jobs(jobs):
    result = {}
    for label, matcher in GROUPS.items():
        matched = [job for job in jobs if matcher(job.get("name", ""))]
        if not matched:
            raise NotifyError(f"Release run contained no jobs for expected {label} group")
        conclusions = [job.get("conclusion") or job.get("status") for job in matched]
        if any(value in {"failure", "timed_out", "action_required", "stale"} for value in conclusions):
            result[label] = "failure"
        elif "cancelled" in conclusions:
            result[label] = "cancelled"
        elif any(value in {"queued", "in_progress", "waiting", "pending", None} for value in conclusions):
            result[label] = "running"
        elif all(value in {"success", "skipped", "neutral"} for value in conclusions):
            if label != "Deploy" and "success" not in conclusions:
                raise NotifyError(
                    f"Release run skipped every job in required {label} group"
                )
            # Deploy jobs are optional when their destination credentials are
            # absent. Every other group must contain at least one real success.
            result[label] = "success"
        else:
            unknown = sorted({str(value) for value in conclusions} - {
                "success", "skipped", "neutral",
            })
            raise NotifyError(
                f"Release run returned unknown {label} conclusions: {unknown}"
            )
    return result


def release_versions(full_version):
    candidates = [full_version]
    stable = full_version.split("-", 1)[0]
    if stable != full_version:
        candidates.append(stable)
    return candidates


def changelog_changes(path, full_version):
    text = Path(path).read_text(encoding="utf-8")
    candidates = release_versions(full_version)
    for version in candidates:
        match = re.search(
            rf"^## \[{re.escape(version)}\].*?\n(.*?)(?=^## \[|\Z)", text, re.MULTILINE | re.DOTALL
        )
        if match:
            lines = match.group(1).strip().splitlines()[:20]
            return "\n".join(lines).strip(), version
    return "", full_version


def article_body(directory, versions):
    for version in versions:
        path = Path(directory) / f"release-{version}.md"
        if not path.is_file():
            continue
        lines = path.read_text(encoding="utf-8").splitlines()
        boundaries = [index for index, line in enumerate(lines) if line.strip() == "---"]
        if len(boundaries) < 2:
            raise NotifyError(f"{path}: missing complete front matter")
        body = lines[boundaries[1] + 1 :]
        if body and body[-1].strip() == "```":
            body.pop()
        return "\n".join(body).strip()
    return None


def emoji(status):
    return "✅" if status == "success" else "❌"


def utf16_units(text):
    return len(text.encode("utf-16-le")) // 2


def truncate_utf16(text, limit):
    result = []
    used = 0
    for character in text:
        width = 2 if ord(character) > 0xFFFF else 1
        if used + width > limit:
            break
        result.append(character)
        used += width
    return "".join(result)


def discord_content(tag, changes, statuses, run_conclusion, repository):
    succeeded = sum(value == "success" for value in statuses.values())
    all_green = succeeded == len(statuses) and run_conclusion == "success"
    header = (
        f"🚀 **LibreFang {tag} Released!**"
        if all_green
        else f"⚠️ **LibreFang {tag} Release — {succeeded}/{len(statuses)} publish groups succeeded**"
    )
    release_url = f"https://github.com/{repository}/releases/tag/{tag}"
    footer = (
        f"📦 [Download]({release_url}) | "
        f"📖 [Changelog](https://github.com/{repository}/blob/main/CHANGELOG.md) | "
        f"⭐ [Star us](https://github.com/{repository})"
    )
    status_lines = [f"{emoji(run_conclusion)} Unified Release workflow"]
    status_lines.extend(f"{emoji(value)} {label}" for label, value in statuses.items())
    fixed = f"{header}\n\n{{changes}}\n\n**Build Status:**\n" + "\n".join(status_lines) + f"\n\n{footer}"
    fallback = f"See [CHANGELOG](https://github.com/{repository}/blob/main/CHANGELOG.md) for details."
    changes = changes or fallback
    budget = 2000 - utf16_units(fixed.format(changes=""))
    if utf16_units(changes) > budget:
        suffix = f"\n\n_…truncated — see the [CHANGELOG](https://github.com/{repository}/blob/main/CHANGELOG.md)._"
        prefix = truncate_utf16(changes, max(0, budget - utf16_units(suffix)))
        if "\n" in prefix:
            prefix = prefix.rsplit("\n", 1)[0]
        changes = prefix + suffix
    content = fixed.format(changes=changes)
    if utf16_units(content) > 2000:
        raise NotifyError("Discord content still exceeds 2000 UTF-16 units")
    return content


def bluesky_text(tag, changes, release_url):
    header = f"LibreFang {tag} released!\n\n"
    footer = f"\n\n{release_url}"
    budget = 300 - len(header) - len(footer)
    summary = "\n".join(changes.splitlines()[:5]).strip()
    if len(summary) > budget:
        summary = summary[: max(0, budget - 1)].rstrip() + "…"
    text = header + summary + footer
    if len(text) > 300 or not text.endswith(release_url):
        raise NotifyError("Bluesky text contract failed")
    start = len(text[: text.index(release_url)].encode("utf-8"))
    end = start + len(release_url.encode("utf-8"))
    return text, start, end


class Notifier:
    def __init__(self, env, http=None):
        self.env = env
        self.http = http or HttpClient()
        self.github = GitHub(env["GITHUB_REPOSITORY"], env["GH_TOKEN"], self.http)
        self.sha = env["WORKFLOW_SHA"]
        self.tag = env["WORKFLOW_BRANCH"]

    def run(self):
        if not re.fullmatch(r"v[^\s]+", self.tag):
            print(f"Not a release tag ({self.tag}), skipping")
            return
        full_version = self.tag[1:]
        source_root = Path(self.env.get("SOURCE_ROOT", "."))
        changes, _ = changelog_changes(source_root / "CHANGELOG.md", full_version)
        jobs = self.github.run_jobs(self.env["WORKFLOW_RUN_ID"])
        statuses = aggregate_jobs(jobs)
        if any(value == "running" for value in statuses.values()):
            raise NotifyError("completed Release run still contains running jobs")

        self.notify_discord(changes, statuses)
        self.notify_bluesky(changes)
        self.notify_discussion(release_versions(full_version))

    def once(self, channel, callback):
        marker = f"{channel}/{self.tag}"
        if len(f"release-notify/{marker}") > 100:
            raise NotifyError(f"release notification marker is too long: {marker}")
        if self.github.marker_done(self.sha, marker):
            print(f"✓ {channel} already notified; skipping")
            return
        description = callback()
        if description:
            self.github.mark_done(self.sha, marker, description)

    def notify_discord(self, changes, statuses):
        webhook = self.env.get("DISCORD_WEBHOOK_URL", "")
        if not webhook:
            print("No Discord release webhook configured, skipping Discord")
            return

        def send():
            content = discord_content(
                self.tag,
                changes,
                statuses,
                self.env["WORKFLOW_CONCLUSION"],
                self.env["GITHUB_REPOSITORY"],
            )
            status, body = self.http.request(
                "POST", webhook, headers={"Content-Type": "application/json"},
                payload={"content": content, "allowed_mentions": {"parse": []}},
            )
            if not 200 <= status < 300:
                raise NotifyError(f"Discord webhook returned HTTP {status}: {body[:500]}")
            print("✓ Discord notification sent")
            return f"Discord notification sent for {self.tag}"

        self.once("discord", send)

    def notify_bluesky(self, changes):
        handle = self.env.get("BLUESKY_HANDLE", "")
        password = self.env.get("BLUESKY_APP_PASSWORD", "")
        if not handle or not password:
            print("No Bluesky credentials configured, skipping Bluesky")
            return

        def send():
            status, body = self.http.request(
                "POST", "https://bsky.social/xrpc/com.atproto.server.createSession",
                headers={"Content-Type": "application/json"},
                payload={"identifier": handle, "password": password},
            )
            session = checked_json("Bluesky authentication", status, body)
            token, did = session.get("accessJwt"), session.get("did")
            if not token or not did:
                raise NotifyError("Bluesky authentication response omitted token or DID")
            release_url = f"https://github.com/{self.env['GITHUB_REPOSITORY']}/releases/tag/{self.tag}"
            text, start, end = bluesky_text(self.tag, changes, release_url)
            payload = {
                "repo": did,
                "collection": "app.bsky.feed.post",
                "record": {
                    "$type": "app.bsky.feed.post",
                    "text": text,
                    "createdAt": self.env["NOW"],
                    "facets": [{
                        "index": {"byteStart": start, "byteEnd": end},
                        "features": [{"$type": "app.bsky.richtext.facet#link", "uri": release_url}],
                    }],
                },
            }
            status, body = self.http.request(
                "POST", "https://bsky.social/xrpc/com.atproto.repo.createRecord",
                headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"},
                payload=payload,
            )
            record = checked_json("Bluesky createRecord", status, body)
            if not record.get("uri") or not record.get("cid"):
                raise NotifyError("Bluesky createRecord response omitted URI or CID")
            print(f"✓ Bluesky post published: {record['uri']}")
            return f"Bluesky notification sent for {self.tag}"

        self.once("bluesky", send)

    def notify_discussion(self, versions):
        body = article_body(Path(self.env.get("SOURCE_ROOT", ".")) / "articles", versions)
        if body is None:
            print("No matching release article found, skipping Discussion")
            return
        title = f"LibreFang {self.tag} Released"

        def send():
            repo_id, category_id, existing = self.github.discussion_target(title)
            url = existing or self.github.create_discussion(repo_id, category_id, title, body)
            print(f"✓ Discussion {'already exists' if existing else 'created'}: {url}")
            return f"Discussion notification sent for {self.tag}"

        self.once("discussion", send)


def checked_json(label, status, body):
    if not 200 <= status < 300:
        raise NotifyError(f"{label} returned HTTP {status}: {body[:500]}")
    try:
        data = json.loads(body)
    except json.JSONDecodeError as exc:
        raise NotifyError(f"{label} returned invalid JSON") from exc
    if not isinstance(data, dict):
        raise NotifyError(f"{label} returned a non-object JSON response")
    if data.get("error"):
        raise NotifyError(f"{label} failed: {data['error']}: {data.get('message', '')}")
    return data


if __name__ == "__main__":
    try:
        Notifier(os.environ).run()
    except (NotifyError, KeyError) as exc:
        print(f"release notification failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
