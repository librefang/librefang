import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// "Cantidades: valores sugeridos + un campo custom, nunca un número libre
// pelado."
//
// Every numeric field the manifest accepts is a quantity with a scale — a
// token budget, a byte quota, a heartbeat cadence — and a bare number box asks
// the operator to already know that scale. The fix was `StepLadderInput`,
// which shows the scale as rungs and keeps the exception reachable.
//
// That is a property of the *form*, not of any one field, so it is checked
// here by scanning the form rather than asserted one field at a time: a new
// quota added next year with a plain `<input type="number">` would otherwise
// pass every test in the suite, because it would render perfectly well.
//
// Scans only `AgentManifestForm.tsx`. `StepLadderInput` owns a
// `type="number"` of its own — the custom-entry field — and that one is
// correct by construction.

const FORM = join(__dirname, "..", "..", "components", "AgentManifestForm.tsx");

/**
 * Fields allowed to keep a bare number input, each with the reason.
 *
 * An entry here is a claim that the ladder is the wrong control for that
 * field, not a field nobody got to yet.
 */
const BARE_INPUT_ALLOWED: Record<string, string> = {
  check_interval_secs:
    "required and validated: continuous mode rejects anything that is not a " +
    "positive integer, and the field carries the error association " +
    "(aria-describedby -> the interval message). A ladder leads with an " +
    "`inherit` rung, which for this field is the invalid state.",
};

describe("manifest quantity presentation", () => {
  it("renders every quantity as a ladder, or says why not", () => {
    const lines = readFileSync(FORM, "utf8").split("\n");

    const offenders: string[] = [];
    for (let i = 0; i < lines.length; i++) {
      if (!lines[i].includes('type="number"')) continue;

      // The field this input belongs to is the nearest label above it.
      let label: string | null = null;
      for (let j = i; j >= 0 && j > i - 30; j--) {
        const m = /label=\{t\("agents\.form\.(\w+)"\)\}/.exec(lines[j]);
        if (m) {
          label = m[1];
          break;
        }
      }

      if (label === null) {
        offenders.push(`L${i + 1}: input with no enclosing Field label`);
        continue;
      }
      if (!(label in BARE_INPUT_ALLOWED)) {
        offenders.push(`L${i + 1}: ${label}`);
      }
    }

    expect(
      offenders,
      `These manifest quantities render as a bare number input. Give them a ` +
        `ladder from src/lib/quantityLadders.ts, or add the field to ` +
        `BARE_INPUT_ALLOWED with the reason the ladder is wrong for it.\n\n` +
        `Bare inputs (${offenders.length}):\n${offenders.join("\n")}`,
    ).toEqual([]);
  });

  it("keeps no stale exemption", () => {
    const source = readFileSync(FORM, "utf8");
    const stale = Object.keys(BARE_INPUT_ALLOWED).filter(
      (field) => !source.includes(`agents.form.${field}`),
    );

    expect(
      stale,
      `BARE_INPUT_ALLOWED exempts fields the form no longer has, so the ` +
        `exemption is no longer about anything.\n\nStale: ${stale.join(", ")}`,
    ).toEqual([]);
  });
});
