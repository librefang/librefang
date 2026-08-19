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
  const fields = log.split('\0');
  if (fields.at(-1) === '') fields.pop();
  if (fields.length % 3 !== 0) {
    throw new Error('git log output did not contain SHA/subject/body triples');
  }
  for (let index = 0; index < fields.length; index += 3) {
    const [sha, subject, body] = fields.slice(index, index + 3);
    if (!/^[0-9a-f]{40,64}$/i.test(sha)) {
      throw new Error(`git log returned an invalid commit id: ${JSON.stringify(sha)}`);
    }
    for (const match of `${subject}\n${body}`.matchAll(REF_REGEX)) {
      const issueNumber = Number.parseInt(match[2], 10);
      if (Number.isSafeInteger(issueNumber) && issueNumber > 0 && !refs.has(issueNumber)) {
        refs.set(issueNumber, { sha: sha.slice(0, 7), subject });
      }
    }
  }
  return refs;
}

function sanitizeInlineCode(value) {
  return String(value)
    .replace(/[\u0000-\u001f\u007f\u2028\u2029]+/g, ' ')
    .replaceAll('`', "'")
    .replaceAll('@', '@\u200b');
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
  sanitizeInlineCode,
};
