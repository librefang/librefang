"""
JSON Formatter Skill — pretty-prints JSON with optional key sorting.

Accepts a raw JSON string and returns it formatted with configurable
indentation and alphabetical key ordering.
"""

import json


def run(input: dict) -> str:
    json_text = input.get("json_text", "")
    sort_keys = input.get("sort_keys", False)
    indent = input.get("indent", 2)

    if not json_text.strip():
        return "Error: empty JSON input"

    # Clamp indent to a reasonable range
    if not isinstance(indent, int) or indent < 0:
        indent = 2
    indent = min(indent, 8)

    try:
        parsed = json.loads(json_text)
    except json.JSONDecodeError as exc:
        return f"Error: invalid JSON — {exc}"

    return json.dumps(parsed, indent=indent, sort_keys=sort_keys, ensure_ascii=False)
