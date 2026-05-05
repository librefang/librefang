# Benign helper — must NOT be flagged by check-skills-supply-chain.py.
# Used by --self-test to confirm the script doesn't have a trigger-happy
# false-positive rate on idiomatic Python skill code.
import re


def normalise(text: str) -> str:
    """Collapse whitespace runs in `text`."""
    return re.sub(r"\s+", " ", text).strip()


def run(payload: dict) -> str:
    return normalise(payload.get("text", ""))
