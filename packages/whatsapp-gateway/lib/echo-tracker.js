'use strict';

/**
 * EchoTracker — process-local LRU for self-loop prevention (EB-01, Phase 3 §A).
 *
 * Records the normalized body of every outbound text the gateway sends via
 * `sock.sendMessage({ text })`. Before forwarding an inbound message to
 * librefang, callers consult `isEcho(body)` to detect and drop the
 * WhatsApp-reflected copy of our own outgoing text (sync/cross-device mirror).
 *
 * Decisions (see .planning/phases/03-chat-isolation-layer/03-CONTEXT.md §A):
 * - In-memory only, no persistence (Q6 locked).
 * - maxSize=100 default, LRU eviction on insertion-order.
 * - Normalization: lowercase + emoji strip + whitespace collapse + trailing
 *   punctuation strip so minor echo rewrites still match.
 */
class EchoTracker {
  constructor(maxSize = 100) {
    this.max = Math.max(1, Number(maxSize) || 100);
    this.map = new Map();
    this.lastSentAt = 0;
  }

  static normalize(body) {
    if (body === null || body === undefined) return '';
    return String(body)
      .toLowerCase()
      .replace(/\p{Extended_Pictographic}/gu, '')
      .replace(/\s+/g, ' ')
      .trim()
      .replace(/[.!?]+$/, '');
  }

  track(body) {
    const key = EchoTracker.normalize(body);
    if (!key) return;
    // Refresh insertion order on re-track.
    if (this.map.has(key)) this.map.delete(key);
    this.map.set(key, Date.now());
    this.lastSentAt = Date.now();
    while (this.map.size > this.max) {
      const oldest = this.map.keys().next().value;
      this.map.delete(oldest);
    }
  }

  isEcho(body) {
    const key = EchoTracker.normalize(body);
    if (!key) return false;
    return this.map.has(key);
  }

  size() {
    return this.map.size;
  }

  elapsedSinceLastSent() {
    return this.lastSentAt ? Date.now() - this.lastSentAt : -1;
  }

  reset() {
    this.map.clear();
    this.lastSentAt = 0;
  }
}

module.exports = { EchoTracker };
