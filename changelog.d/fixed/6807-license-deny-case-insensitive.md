Make `xtask license-check --deny` match denied SPDX license ids case-insensitively.
The denied-list comparison used exact string equality against the canonical SPDX id, so a custom `--deny` entry with different casing than the canonical form (e.g. `gpl-3.0-only` vs `GPL-3.0-only`) silently failed to match and let the license through (#6807) (@houko)
