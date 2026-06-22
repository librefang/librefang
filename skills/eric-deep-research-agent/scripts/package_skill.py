#!/usr/bin/env python3
"""
Package script for DeepResearch Agent skill.
Creates a distributable .skill archive.
"""

import os
import sys
import json
import shutil
import zipfile
from pathlib import Path
from datetime import datetime

SKILL_DIR = Path(__file__).parent.parent
OUTPUT_DIR = Path("/workspace/temp-skills/output")

def get_skill_metadata():
    """Read skill metadata from _meta.json."""
    meta_path = SKILL_DIR / "_meta.json"
    if not meta_path.exists():
        raise FileNotFoundError("_meta.json not found")

    return json.loads(meta_path.read_text(encoding="utf-8"))

def create_package():
    """Create the .skill package."""
    meta = get_skill_metadata()
    skill_name = SKILL_DIR.name
    version = meta.get("version", "1.0.0")
    skill_id = meta.get("id", "unknown")

    # Create output directory
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)

    # Generate output filename
    output_file = OUTPUT_DIR / f"{skill_name}_v{version}_{skill_id}.skill"

    # Create zip archive
    with zipfile.ZipFile(output_file, 'w', zipfile.ZIP_DEFLATED) as zf:
        for root, dirs, files in os.walk(SKILL_DIR):
            # Skip __pycache__ and other cache directories
            dirs[:] = [d for d in dirs if d not in ['__pycache__', '.git', '.venv', 'node_modules']]

            for file in files:
                # Skip unnecessary files
                if file.endswith('.pyc') or file.startswith('.'):
                    continue

                file_path = Path(root) / file
                arcname = file_path.relative_to(SKILL_DIR)
                zf.write(file_path, arcname)

    print(f"Package created: {output_file}")
    print(f"Skill ID: {skill_id}")
    print(f"Version: {version}")

    # Verify contents
    with zipfile.ZipFile(output_file, 'r') as zf:
        print("\nPackage contents:")
        for info in zf.infolist():
            print(f"  - {info.filename}")

    return output_file

def main():
    """Main entry point."""
    try:
        output_path = create_package()
        print(f"\nSuccessfully packaged skill to: {output_path}")
        return 0
    except Exception as e:
        print(f"Error packaging skill: {e}", file=sys.stderr)
        return 1

if __name__ == "__main__":
    sys.exit(main())
