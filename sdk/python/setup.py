import re
from pathlib import Path

from setuptools import find_packages, setup


PYPROJECT = Path(__file__).with_name("pyproject.toml")
PROJECT_VERSION = re.search(
    r'^version\s*=\s*"([^"]+)"',
    PYPROJECT.read_text(encoding="utf-8"),
    re.MULTILINE,
)
if PROJECT_VERSION is None:
    raise RuntimeError("project version is missing from pyproject.toml")

setup(
    name="librefang-sdk",
    version=PROJECT_VERSION.group(1),
    description="Official Python client for the LibreFang Agent OS REST API",
    packages=find_packages(include=("librefang", "librefang.*")),
    package_data={"librefang": ["sidecar/template/*", "sidecar/template/**/*"]},
    python_requires=">=3.10",
    classifiers=[
        "Programming Language :: Python :: 3",
        "License :: OSI Approved :: MIT License",
        "Operating System :: OS Independent",
    ],
)
