#!/usr/bin/env python3
"""Regression checks for the credentialed AUR publishing boundary."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
ACTION = ROOT / ".github/actions/publish-aur/action.yml"
PINNED_IMAGE = (
    "archlinux:base-devel@"
    "sha256:ee205c220399524a683cf495d411691b921baed8ab47cdc6d732efa782fae484"
)


class PublishAurActionHardeningTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.action = ACTION.read_text(encoding="utf-8")

    def test_host_keys_are_required_instead_of_acquired_on_runner(self) -> None:
        self.assertNotIn("ssh-keyscan", self.action)
        self.assertIn('if [[ -z "${AUR_KNOWN_HOSTS//[[:space:]]/}" ]]', self.action)
        self.assertIn("AUR_KNOWN_HOSTS must pin aur.archlinux.org", self.action)

    def test_credential_files_use_restrictive_permissions(self) -> None:
        self.assertIn(
            '( umask 077; printf \'%s\\n\' "$AUR_SSH_PRIVATE_KEY" > "$KEY" )',
            self.action,
        )
        self.assertIn(
            '( umask 077; printf \'%s\\n\' "$AUR_KNOWN_HOSTS" > "$KNOWN" )',
            self.action,
        )

    def test_publish_container_has_read_only_source_and_pinned_identity(self) -> None:
        self.assertIn('-v "$GITHUB_WORKSPACE":/repo:ro', self.action)
        self.assertIn(PINNED_IMAGE, self.action)
        self.assertNotIn("archlinux:base-devel \\", self.action)


if __name__ == "__main__":
    unittest.main()
