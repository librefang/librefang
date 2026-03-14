"""
Word Counter Skill — a minimal example of a Python skill for LibreFang.

This skill receives a text input and returns word/sentence/character counts.
"""

import re


def run(input: dict) -> str:
    text = input.get("text", "")

    words = len(text.split())
    sentences = len(re.split(r"[.!?]+", text.strip())) if text.strip() else 0
    characters = len(text)

    return (
        f"Words: {words}\n"
        f"Sentences: {sentences}\n"
        f"Characters: {characters}"
    )
