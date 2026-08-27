#!/usr/bin/env python3

import importlib.util
import json
import tempfile
import unittest
import urllib.error
from unittest import mock
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("release_notify.py")
SPEC = importlib.util.spec_from_file_location("release_notify", MODULE_PATH)
release_notify = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release_notify)


class ReleaseNotifyTests(unittest.TestCase):
    class FakeHttp:
        def __init__(self, responses):
            self.responses = list(responses)
            self.calls = []

        def request(self, method, url, *, headers=None, payload=None):
            self.calls.append((method, url, payload))
            return self.responses.pop(0)

    def representative_jobs(self):
        names = [
            "CLI / x86_64-unknown-linux-gnu",
            "Sign Release Artifacts",
            "Sync to Homebrew Tap",
            "Publish AUR / librefang-bin",
            "Publish pacman repo (Arch)",
            "Desktop / Linux x86_64",
            "Mobile / iOS (ipa)",
            "Sync Homebrew Cask",
            "Publish AUR / librefang-desktop-bin",
            "Docker / linux/amd64",
            "Docker Scan / linux/amd64",
            "Docker Manifest",
            "Publish AUR / librefang-docker",
            "Deploy to Fly.io",
            "Deploy to Render",
            "SDK / JavaScript (npm)",
            "SDK / Python (PyPI)",
        ]
        return [{"name": name, "status": "completed", "conclusion": "success"} for name in names]

    def test_aggregate_covers_unified_release_groups_and_failures(self):
        jobs = self.representative_jobs()
        next(job for job in jobs if job["name"] == "Mobile / iOS (ipa)")["conclusion"] = "failure"
        next(job for job in jobs if job["name"] == "Docker Scan / linux/amd64")["conclusion"] = "skipped"

        statuses = release_notify.aggregate_jobs(jobs)

        self.assertEqual("failure", statuses["Desktop"])
        self.assertEqual("success", statuses["Docker"])
        self.assertEqual("success", statuses["CLI"])
        self.assertEqual("success", statuses["Deploy"])
        self.assertEqual("success", statuses["SDK"])

    def test_aggregate_rejects_unknown_and_fully_skipped_required_groups(self):
        jobs = self.representative_jobs()
        for job in jobs:
            if job["name"].startswith("SDK /"):
                job["conclusion"] = "skipped"
        with self.assertRaisesRegex(release_notify.NotifyError, "required SDK"):
            release_notify.aggregate_jobs(jobs)

        jobs = self.representative_jobs()
        next(job for job in jobs if job["name"].startswith("CLI /"))[
            "conclusion"
        ] = "startup_failure"
        with self.assertRaisesRegex(release_notify.NotifyError, "unknown CLI"):
            release_notify.aggregate_jobs(jobs)

        jobs = self.representative_jobs()
        for job in jobs:
            if job["name"].startswith("Deploy to "):
                job["conclusion"] = "skipped"
        self.assertEqual("success", release_notify.aggregate_jobs(jobs)["Deploy"])

    def test_missing_expected_group_fails_loudly(self):
        jobs = [job for job in self.representative_jobs() if not job["name"].startswith("SDK /")]
        with self.assertRaises(release_notify.NotifyError):
            release_notify.aggregate_jobs(jobs)

    def test_github_job_inventory_paginates_and_propagates_http_errors(self):
        http = self.FakeHttp([
            (200, json.dumps({"jobs": [{"name": "one"}, {"name": "two"}]})),
            (200, json.dumps({"jobs": [{"name": "three"}]})),
        ])
        github = release_notify.GitHub("owner/repo", "token", http)
        self.assertEqual(3, len(github.run_jobs("42", page_size=2)))
        self.assertIn("page=2", http.calls[1][1])

        failing = release_notify.GitHub(
            "owner/repo", "token", self.FakeHttp([(503, '{"message":"unavailable"}')])
        )
        with self.assertRaises(release_notify.NotifyError):
            failing.run_jobs("42")

    def test_status_marker_inventory_paginates_and_validates_shape(self):
        first_page = [
            {"context": "unrelated", "state": "success"},
            {"context": "release-notify/discord/v1", "state": "failure"},
        ]
        second_page = [
            {"context": "release-notify/discord/v1", "state": "success"}
        ]
        http = self.FakeHttp(
            [(200, json.dumps(first_page)), (200, json.dumps(second_page))]
        )
        github = release_notify.GitHub("owner/repo", "token", http)
        self.assertTrue(github.marker_done("abc", "discord/v1", page_size=2))
        self.assertIn("page=2", http.calls[1][1])

        malformed = release_notify.GitHub(
            "owner/repo", "token", self.FakeHttp([(200, '{"statuses":[]}')])
        )
        with self.assertRaisesRegex(release_notify.NotifyError, "not an array"):
            malformed.marker_done("abc", "discord/v1")

    def test_discussion_target_resolves_runtime_ids_and_paginates(self):
        def page(nodes, has_next, cursor):
            return json.dumps({
                "data": {"repository": {
                    "id": "repo-id",
                    "discussionCategories": {"nodes": [
                        {"id": "category-id", "name": "Announcements"}
                    ]},
                    "discussions": {
                        "nodes": nodes,
                        "pageInfo": {"hasNextPage": has_next, "endCursor": cursor},
                    },
                }}
            })

        http = self.FakeHttp([
            (200, page([{"title": "Other", "url": "https://example/other"}], True, "next")),
            (200, page([{"title": "Wanted", "url": "https://example/wanted"}], False, None)),
        ])
        github = release_notify.GitHub("owner/repo", "token", http)
        self.assertEqual(
            ("repo-id", "category-id", "https://example/wanted"),
            github.discussion_target("Wanted"),
        )
        self.assertIsNone(http.calls[0][2]["variables"]["after"])
        self.assertEqual("next", http.calls[1][2]["variables"]["after"])

    def test_prerelease_changelog_prefers_full_version_then_stable_fallback(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "CHANGELOG.md"
            path.write_text(
                "# Changelog\n\n## [1.2.3-beta.1]\n\n- beta detail\n\n## [1.2.3]\n\n- stable detail\n",
                encoding="utf-8",
            )
            changes, version = release_notify.changelog_changes(path, "1.2.3-beta.1")
            self.assertEqual("- beta detail", changes)
            self.assertEqual("1.2.3-beta.1", version)

            path.write_text("## [1.2.3]\n\n- stable detail\n", encoding="utf-8")
            changes, version = release_notify.changelog_changes(path, "1.2.3-beta.1")
            self.assertEqual("- stable detail", changes)
            self.assertEqual("1.2.3", version)

        self.assertEqual(
            ["1.2.3-beta.1", "1.2.3"],
            release_notify.release_versions("1.2.3-beta.1"),
        )

    def test_article_body_uses_prerelease_file_and_strips_outer_fence(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "release-1.2.3-beta.1.md"
            path.write_text(
                "```markdown\n---\ntitle: Beta\npublished: true\n---\nBody\n```\n",
                encoding="utf-8",
            )
            self.assertEqual(
                "Body", release_notify.article_body(directory, ["1.2.3-beta.1", "1.2.3"])
            )

    def test_discord_content_is_bounded_and_reports_unified_status(self):
        statuses = {name: "success" for name in release_notify.GROUPS}
        statuses["Desktop"] = "failure"
        content = release_notify.discord_content(
            "v1.2.3-beta.1", "- very long change\n" * 500, statuses, "failure", "owner/repo"
        )
        self.assertLessEqual(len(content), 2000)
        self.assertIn("Unified Release workflow", content)
        self.assertIn("❌ Desktop", content)
        self.assertIn("truncated", content)

        emoji_content = release_notify.discord_content(
            "v1.2.3", "🚀" * 2000, statuses, "failure", "owner/repo"
        )
        self.assertLessEqual(release_notify.utf16_units(emoji_content), 2000)
        self.assertIn("truncated", emoji_content)

    def test_network_errors_redact_webhook_credentials(self):
        webhook = "https://discord.com/api/webhooks/123/super-secret-token"
        with mock.patch(
            "urllib.request.urlopen",
            side_effect=urllib.error.URLError("connection reset"),
        ):
            with self.assertRaises(release_notify.NotifyError) as raised:
                release_notify.HttpClient().request("POST", webhook, payload={})
        message = str(raised.exception)
        self.assertIn("https://discord.com", message)
        self.assertNotIn("123", message)
        self.assertNotIn("super-secret-token", message)

    def test_bluesky_truncation_preserves_url_and_utf8_facet(self):
        url = "https://github.com/owner/repo/releases/tag/v1.2.3-beta.1"
        text, start, end = release_notify.bluesky_text(
            "v1.2.3-beta.1", "🚀 composed grapheme e\u0301 " * 100, url
        )
        encoded = text.encode("utf-8")
        self.assertLessEqual(len(text), 300)
        self.assertTrue(text.endswith(url))
        self.assertEqual(url, encoded[start:end].decode())

    def test_checked_json_rejects_http_api_and_shape_failures(self):
        with self.assertRaises(release_notify.NotifyError):
            release_notify.checked_json("Bluesky", 401, '{"error":"AuthRequired"}')
        with self.assertRaises(release_notify.NotifyError):
            release_notify.checked_json(
                "Bluesky", 200, '{"error":"RateLimitExceeded","message":"slow down"}'
            )
        with self.assertRaises(release_notify.NotifyError):
            release_notify.checked_json("Bluesky", 200, "not json")

    def test_marker_is_tag_scoped_and_skips_completed_channel(self):
        class FakeGitHub:
            def __init__(self):
                self.checked = []
                self.marked = []

            def marker_done(self, sha, marker):
                self.checked.append((sha, marker))
                return marker == "discord/v1.2.3"

            def mark_done(self, sha, marker, description):
                self.marked.append((sha, marker, description))

        notifier = object.__new__(release_notify.Notifier)
        notifier.github = FakeGitHub()
        notifier.sha = "abc123"
        notifier.tag = "v1.2.3"
        called = []

        notifier.once("discord", lambda: called.append(True))
        notifier.once("bluesky", lambda: "posted")

        self.assertEqual([], called)
        self.assertEqual(
            [("abc123", "discord/v1.2.3"), ("abc123", "bluesky/v1.2.3")],
            notifier.github.checked,
        )
        self.assertEqual(
            [("abc123", "bluesky/v1.2.3", "posted")], notifier.github.marked
        )


if __name__ == "__main__":
    unittest.main()
