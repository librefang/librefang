'use strict';

/**
 * Message-ID deduplication tracker.
 *
 * Two-phase API — `wasProcessed` and `markProcessed` are distinct:
 * - Baileys re-emits `messages.upsert` for a given id whenever the previous
 *   handling ended with a decrypt failure (null payload, SessionError,
 *   PreKeyError). The retransmit is the ONLY opportunity for the gateway
 *   to call `assertSessions` and recover the Signal session.
 * - A tracker that marks on first sight blocks the retransmit and strands
 *   the sender permanently until manual key re-scan. That was the 2026-04-16
 *   outage with the Signore's own chat.
 *
 * The fix is structural: callers must decide when a message has been
 * "really" processed and call `markProcessed` only then. Typical policy:
 *   - decrypt succeeded (`msg.message != null`) → mark
 *   - decrypt failed → leave unmarked so the retransmit reaches recovery
 *
 * The tracker prunes entries older than `windowMs` on each check (lazy);
 * WhatsApp's own retransmit window is a few seconds, so 60 s is generous.
 */
function createDedupTracker({ windowMs = 60_000, now = () => Date.now() } = {}) {
  const seen = new Map(); // id → timestamp

  function prune(nowMs = now()) {
    for (const [id, ts] of seen) {
      if (nowMs - ts > windowMs) seen.delete(id);
    }
  }

  return {
    wasProcessed(id) {
      if (!id) return false;
      prune();
      return seen.has(id);
    },
    markProcessed(id) {
      if (!id) return;
      seen.set(id, now());
    },
    unmarkProcessed(id) {
      if (!id) return;
      seen.delete(id);
    },
    size() {
      return seen.size;
    },
    // Exposed for tests.
    _prune: prune,
  };
}

module.exports = { createDedupTracker };
