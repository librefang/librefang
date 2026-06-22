#!/usr/bin/env python3
"""
Quick validation script for DeepResearch Agent skill.
Validates frontmatter, structure, and required files.
"""

import os
import sys
import json
import re
from pathlib import Path

SKILL_DIR = Path(__file__).parent.parent
REQUIRED_FILES = ["SKILL.md", "_meta.json"]
REQUIRED_META_KEYS = ["id", "version"]

def validate_frontmatter():
    """Validate SKILL.md frontmatter format."""
    skill_md = SKILL_DIR / "SKILL.md"
    if not skill_md.exists():
        return False, "SKILL.md not found"

    content = skill_md.read_text(encoding="utf-8")

    # Check if file starts with frontmatter
    if not content.strip().startswith("---"):
        return False, "SKILL.md must start with YAML frontmatter (---)"

    # Extract frontmatter
    parts = content.split("---", 2)
    if len(parts) < 3:
        return False, "Invalid frontmatter format"

    frontmatter = parts[1]

    # Check required fields
    required_fields = ["name", "description"]
    for field in required_fields:
        if not re.search(rf"^{field}:", frontmatter, re.MULTILINE):
            return False, f"Missing required frontmatter field: {field}"

    # Validate name format (hyphen-case)
    name_match = re.search(r"^name:\s*(.+)$", frontmatter, re.MULTILINE)
    if name_match:
        name = name_match.group(1).strip()
        if not re.match(r"^[a-z0-9]+(-[a-z0-9]+)*$", name):
            return False, f"Name must be hyphen-case: '{name}'"

    return True, "Frontmatter valid"

def validate_meta_json():
    """Validate _meta.json structure."""
    meta_json = SKILL_DIR / "_meta.json"
    if not meta_json.exists():
        return False, "_meta.json not found"

    try:
        data = json.loads(meta_json.read_text(encoding="utf-8"))

        # Check required keys
        for key in REQUIRED_META_KEYS:
            if key not in data:
                return False, f"Missing required _meta.json key: {key}"

        # Validate version format (semver)
        version = data.get("version", "")
        if not re.match(r"^\d+\.\d+\.\d+$", version):
            return False, f"Invalid version format: {version}"

        # Validate id is integer
        if not isinstance(data.get("id"), int):
            return False, "id must be an integer"

        return True, "_meta.json valid"

    except json.JSONDecodeError as e:
        return False, f"Invalid JSON in _meta.json: {e}"

def validate_directory_structure():
    """Validate skill directory structure."""
    # Check required files exist
    for filename in REQUIRED_FILES:
        filepath = SKILL_DIR / filename
        if not filepath.exists():
            return False, f"Missing required file: {filename}"

    # Check directories exist
    for dirname in ["scripts", "references", "assets"]:
        dirpath = SKILL_DIR / dirname
        if not dirpath.exists():
            return False, f"Missing required directory: {dirname}"

    return True, "Directory structure valid"

def validate_naming():
    """Validate skill folder name matches frontmatter name."""
    skill_md = SKILL_DIR / "SKILL.md"
    content = skill_md.read_text(encoding="utf-8")

    parts = content.split("---", 2)
    frontmatter = parts[1]

    name_match = re.search(r"^name:\s*(.+)$", frontmatter, re.MULTILINE)
    if name_match:
        frontmatter_name = name_match.group(1).strip()
        folder_name = SKILL_DIR.name

        if frontmatter_name != folder_name:
            return False, f"Name mismatch: folder='{folder_name}', frontmatter='{frontmatter_name}'"

    return True, "Naming valid"

def main():
    """Run all validations."""
    checks = [
        ("Frontmatter", validate_frontmatter),
        ("_meta.json", validate_meta_json),
        ("Directory Structure", validate_directory_structure),
        ("Naming", validate_naming),
    ]

    all_passed = True
    for check_name, check_func in checks:
        passed, message = check_func()
        status = "PASS" if passed else "FAIL"
        print(f"[{status}] {check_name}: {message}")
        if not passed:
            all_passed = False

    if all_passed:
        print("\nAll validations passed!")
        return 0
    else:
        print("\nValidation failed. Please fix the issues above.")
        return 1

if __name__ == "__main__":
    sys.exit(main())
