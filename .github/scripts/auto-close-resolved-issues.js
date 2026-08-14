'use strict';

const FLAG_MARKER = '<!-- auto-close-reconciler:flagged -->';
const CLOSE_MARKER = '<!-- auto-close-reconciler:closing -->';

function parseLookback(raw) {
  const lookback = Number(raw);
  if (!Number.isInteger(lookback) || lookback < 1 || lookback > 365) {
    throw new Error(`lookback_days must be an integer from 1 to 365; got ${JSON.stringify(raw)}`);
  }
  return lookback;
}

// GitHub closing keywords, excluding direct negations. JavaScript lookbehind
// must be fixed-width, so contractions are intentionally enumerated.
const REF_REGEX =
  /(?<!\bnot )(?<!\bnever )(?<!\bwon't )(?<!\bwouldn't )(?<!\bcannot )(?<!\bcan't )(?<!\bdon't )(?<!\bdoesn't )(?<!\bdidn't )(?<!\bisn't )(?<!\baren't )(?<!\bwasn't )(?<!\bweren't )(?<!\bcouldn't )(?<!\bshouldn't )(?<!\bhasn't )(?<!\bhaven't )\b(close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s+#(\d+)/gi;

function collectRefs(log) {
  const refs = new Map();
  for (const block of log.split('---COMMIT-END---')) {
    const trimmed = block.trim();
    if (!trimmed) continue;
    const lines = trimmed.split('\n');
    const sha = lines[0];
    const subject = lines[1] || '';
    for (const match of trimmed.matchAll(REF_REGEX)) {
      const issueNumber = Number.parseInt(match[2], 10);
      if (!refs.has(issueNumber)) {
        refs.set(issueNumber, { sha: sha.slice(0, 7), subject });
      }
    }
  }
  return refs;
}

function commentDisposition(live, bodies) {
  const marker = live ? CLOSE_MARKER : FLAG_MARKER;
  const alreadyCommented = bodies.some(
    (body) => typeof body === 'string' && body.includes(marker),
  );
  return {
    skipIssue: !live && alreadyCommented,
    addComment: !alreadyCommented,
  };
}

module.exports = {
  CLOSE_MARKER,
  FLAG_MARKER,
  collectRefs,
  commentDisposition,
  parseLookback,
};
