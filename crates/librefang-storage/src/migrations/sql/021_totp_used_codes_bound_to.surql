-- Migration v21: Add bound_to column to totp_used_codes for TOTP TOCTOU fix (#3360).
--
-- Mirrors SQLite migration v31. Binds a one-time TOTP code to the specific
-- action it was presented for (e.g. "approve:<id>") so that a code used
-- for one approval cannot be replayed to authorize a different action.
--
-- Pre-existing rows will have NULL values — the nil bound_to is treated as
-- "unbound" and accepted for backward compatibility.

DEFINE FIELD IF NOT EXISTS bound_to ON totp_used_codes TYPE option<string>;
