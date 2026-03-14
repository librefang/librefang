#!/usr/bin/env python3
"""Validate all TOML files in the model catalog against the schema.

Usage:
    python scripts/validate.py

Checks:
    - All provider TOML files parse correctly
    - Required provider fields exist
    - Required model fields exist for each [[models]] entry
    - Tier values are one of: frontier, smart, balanced, fast, local
    - Cost values are non-negative
    - context_window and max_output_tokens are positive integers
    - No duplicate model IDs within the same provider file
    - No duplicate model IDs across ALL provider files (same provider)
    - aliases.toml parses correctly

Exit code 0 on success, 1 on any validation error.
"""

import os
import sys
from pathlib import Path

try:
    import tomllib  # Python 3.11+
except ImportError:
    try:
        import tomli as tomllib  # pip install tomli
    except ImportError:
        print("ERROR: Python 3.11+ required, or install 'tomli': pip install tomli")
        sys.exit(1)

VALID_TIERS = {"frontier", "smart", "balanced", "fast", "local"}

REQUIRED_PROVIDER_FIELDS = {"id", "display_name", "api_key_env", "base_url", "key_required"}

REQUIRED_MODEL_FIELDS = {
    "id",
    "display_name",
    "tier",
    "context_window",
    "max_output_tokens",
    "input_cost_per_m",
    "output_cost_per_m",
}


def validate_provider_file(filepath: Path) -> list[str]:
    """Validate a single provider TOML file. Returns list of error messages."""
    errors = []

    try:
        with open(filepath, "rb") as f:
            data = tomllib.load(f)
    except Exception as e:
        return [f"{filepath.name}: Failed to parse TOML: {e}"]

    # Validate [provider] section
    provider = data.get("provider")
    if provider is None:
        errors.append(f"{filepath.name}: Missing [provider] section")
    else:
        for field in REQUIRED_PROVIDER_FIELDS:
            if field not in provider:
                errors.append(f"{filepath.name}: Missing provider field '{field}'")

    # Validate [[models]] entries
    models = data.get("models", [])
    seen_ids = set()

    for i, model in enumerate(models):
        model_label = model.get("id", f"models[{i}]")

        # Check required fields
        for field in REQUIRED_MODEL_FIELDS:
            if field not in model:
                errors.append(f"{filepath.name}: Model '{model_label}' missing field '{field}'")

        # Validate tier
        tier = model.get("tier")
        if tier is not None and tier not in VALID_TIERS:
            errors.append(
                f"{filepath.name}: Model '{model_label}' has invalid tier '{tier}' "
                f"(must be one of: {', '.join(sorted(VALID_TIERS))})"
            )

        # Validate costs
        for cost_field in ("input_cost_per_m", "output_cost_per_m"):
            val = model.get(cost_field)
            if val is not None and (not isinstance(val, (int, float)) or val < 0):
                errors.append(
                    f"{filepath.name}: Model '{model_label}' has invalid {cost_field}: {val} "
                    f"(must be >= 0)"
                )

        # Validate context_window and max_output_tokens
        for int_field in ("context_window", "max_output_tokens"):
            val = model.get(int_field)
            if val is not None and (not isinstance(val, int) or val <= 0):
                errors.append(
                    f"{filepath.name}: Model '{model_label}' has invalid {int_field}: {val} "
                    f"(must be a positive integer)"
                )

        # Check for duplicate IDs within the file
        model_id = model.get("id")
        if model_id:
            if model_id in seen_ids:
                errors.append(
                    f"{filepath.name}: Duplicate model ID '{model_id}' within file"
                )
            seen_ids.add(model_id)

    return errors


def validate_aliases_file(filepath: Path) -> list[str]:
    """Validate aliases.toml. Returns list of error messages."""
    if not filepath.exists():
        return [f"aliases.toml: File not found at {filepath}"]

    try:
        with open(filepath, "rb") as f:
            data = tomllib.load(f)
    except Exception as e:
        return [f"aliases.toml: Failed to parse TOML: {e}"]

    errors = []
    aliases = data.get("aliases", {})
    if not isinstance(aliases, dict):
        errors.append("aliases.toml: [aliases] must be a table of string -> string mappings")
    else:
        for alias, target in aliases.items():
            if not isinstance(target, str):
                errors.append(f"aliases.toml: Alias '{alias}' must map to a string, got {type(target).__name__}")

    return errors


def main():
    # Find the catalog root
    script_dir = Path(__file__).resolve().parent
    catalog_root = script_dir.parent
    providers_dir = catalog_root / "providers"

    if not providers_dir.is_dir():
        print(f"ERROR: providers/ directory not found at {providers_dir}")
        sys.exit(1)

    all_errors = []
    total_models = 0
    provider_counts = {}
    global_model_ids = {}  # model_id -> (provider_id, filename)

    # Validate each provider file
    toml_files = sorted(providers_dir.glob("*.toml"))
    if not toml_files:
        print("ERROR: No .toml files found in providers/")
        sys.exit(1)

    for filepath in toml_files:
        errors = validate_provider_file(filepath)
        all_errors.extend(errors)

        # Count models if file parsed successfully
        if not any("Failed to parse" in e for e in errors):
            try:
                with open(filepath, "rb") as f:
                    data = tomllib.load(f)
                provider_id = data.get("provider", {}).get("id", filepath.stem)
                models = data.get("models", [])
                count = len(models)
                total_models += count
                provider_counts[provider_id] = count

                # Track global model IDs for cross-file duplicate detection
                for model in models:
                    mid = model.get("id")
                    mprov = model.get("provider", provider_id)
                    if mid:
                        key = (mid, mprov)
                        if key in global_model_ids:
                            prev_file = global_model_ids[key]
                            # Only flag if same provider in different files
                            if prev_file != filepath.name:
                                all_errors.append(
                                    f"Cross-file duplicate: model '{mid}' (provider '{mprov}') "
                                    f"found in both {prev_file} and {filepath.name}"
                                )
                        else:
                            global_model_ids[key] = filepath.name
            except Exception:
                pass

    # Validate aliases.toml
    aliases_file = catalog_root / "aliases.toml"
    all_errors.extend(validate_aliases_file(aliases_file))

    # Print results
    print("=" * 60)
    print("LibreFang Model Catalog Validation")
    print("=" * 60)
    print()

    if all_errors:
        print(f"ERRORS ({len(all_errors)}):")
        for error in all_errors:
            print(f"  - {error}")
        print()

    print(f"Provider files: {len(toml_files)}")
    print(f"Total models:   {total_models}")
    print()

    print("Per-provider model counts:")
    for provider_id in sorted(provider_counts.keys()):
        count = provider_counts[provider_id]
        if count > 0:
            print(f"  {provider_id:30s} {count:>3}")

    print()

    if all_errors:
        print(f"VALIDATION FAILED with {len(all_errors)} error(s)")
        sys.exit(1)
    else:
        print("VALIDATION PASSED")
        sys.exit(0)


if __name__ == "__main__":
    main()
