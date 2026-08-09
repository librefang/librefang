import re
import shutil
import subprocess
import sys
from pathlib import Path


def _project_version(pyproject: Path) -> str:
    match = re.search(
        r'^version\s*=\s*"([^"]+)"',
        pyproject.read_text(encoding="utf-8"),
        re.MULTILINE,
    )
    assert match is not None
    return match.group(1)


def test_legacy_setup_metadata_includes_real_package(tmp_path):
    project = Path(__file__).parents[1]
    checkout = tmp_path / "project"
    shutil.copytree(project, checkout)

    egg_base = tmp_path / "egg-info"
    egg_base.mkdir()
    subprocess.run(
        [sys.executable, "setup.py", "egg_info", "--egg-base", str(egg_base)],
        cwd=checkout,
        check=True,
        capture_output=True,
        text=True,
    )

    egg_info = next(egg_base.glob("*.egg-info"))
    metadata = (egg_info / "PKG-INFO").read_text(encoding="utf-8")
    top_level = (egg_info / "top_level.txt").read_text(encoding="utf-8").splitlines()

    assert "Name: librefang-sdk" in metadata
    assert f"Version: {_project_version(checkout / 'pyproject.toml')}" in metadata
    assert top_level == ["librefang"]
