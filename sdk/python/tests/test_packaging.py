import os
import re
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path


def test_legacy_setup_metadata_includes_real_package(tmp_path):
    project = Path(__file__).parents[1]
    checkout = tmp_path / "project"
    checkout.mkdir()
    for filename in ("setup.py", "pyproject.toml", "README.md", "LICENSE"):
        shutil.copy2(project / filename, checkout / filename)
    shutil.copytree(project / "librefang", checkout / "librefang")
    nested_template = checkout / "librefang" / "sidecar" / "template" / "assets" / "config"
    nested_template.mkdir(parents=True)
    (nested_template / "example.toml").write_text("enabled = true\n", encoding="utf-8")

    fixture_version = "2099.1.2rc3"
    pyproject = checkout / "pyproject.toml"
    pyproject.write_text(
        re.sub(
            r'^version\s*=\s*"[^"]+"',
            f'version = "{fixture_version}"',
            pyproject.read_text(encoding="utf-8"),
            count=1,
            flags=re.MULTILINE,
        ),
        encoding="utf-8",
    )

    dist_dir = tmp_path / "dist"
    subprocess.run(
        [sys.executable, "setup.py", "bdist_wheel", "--dist-dir", str(dist_dir)],
        cwd=checkout,
        check=True,
        capture_output=True,
        text=True,
    )

    wheel = next(dist_dir.glob("*.whl"))
    with zipfile.ZipFile(wheel) as archive:
        names = archive.namelist()
        metadata_name = next(name for name in names if name.endswith(".dist-info/METADATA"))
        metadata = archive.read(metadata_name).decode("utf-8")

    assert "Name: librefang-sdk" in metadata
    assert f"Version: {fixture_version}" in metadata
    assert "Requires-Python: >=3.10" in metadata
    assert "librefang/__init__.py" in names
    assert "librefang/sidecar/adapters/discord.py" in names
    assert "librefang/sidecar/template/README.md" in names
    assert "librefang/sidecar/template/adapter.py.tmpl" in names
    assert "librefang/sidecar/template/requirements.txt" in names
    assert "librefang/sidecar/template/assets/config/example.toml" in names

    env = os.environ.copy()
    env["PYTHONPATH"] = str(wheel)
    imported_version = subprocess.run(
        [sys.executable, "-c", "import librefang; print(librefang.__version__)"],
        cwd=tmp_path,
        env=env,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    assert imported_version == fixture_version
