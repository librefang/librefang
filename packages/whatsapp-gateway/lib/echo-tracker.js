'use strict';

/**
 * EchoTracker — process-local LRU for self-loop prevention (EB-01, Phase 3 §A).
 *
 * Records the chat-scoped normalized body of every outbound text the gateway sends via
 * `sock.sendMessage({ text })`. Before forwarding an inbound message to
 * librefang, callers consult `isEcho(body)` to detect and drop the
 * WhatsApp-reflected copy of our own outgoing text (sync/cross-device mirror).
 *
 * Decisions (see .planning/phases/03-chat-isolation-layer/03-CONTEXT.md §A):
 * - In-memory only, no persistence (Q6 locked).
 * - Process-local: run one gateway process per account. Multi-process echo
 *   detection would require a shared tracker.
 * - maxSize=100 default, LRU eviction on insertion-order.
 * - ttlMs=5 minutes default, with lazy expiry on access.
 * - Normalization: lowercase + emoji strip + whitespace collapse + trailing
 *   punctuation strip so minor echo rewrites still match.
 */
class EchoTracker {
  constructor(maxSize = 100, { ttlMs = 300_000, now = () => Date.now() } = {}) {
    this.max = Math.max(1, Number(maxSize) || 100);
    if (typeof ttlMs !== 'number' || !Number.isFinite(ttlMs) || ttlMs <= 0) {
      throw new RangeError(`EchoTracker: ttlMs must be a positive number, got ${ttlMs}`);
    }
    this.ttlMs = ttlMs;
    this.now = now;
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

  key(body, scope = '') {
    const normalized = EchoTracker.normalize(body);
    return normalized ? `${String(scope)}\0${normalized}` : '';
  }

  prune(now = this.now()) {
    for (const [key, timestamp] of this.map) {
      if (now - timestamp > this.ttlMs) this.map.delete(key);
    }
  }

  track(body, scope = '') {
    const key = this.key(body, scope);
    if (!key) return;
    const now = this.now();
    this.prune(now);
    // Refresh insertion order on re-track.
    if (this.map.has(key)) this.map.delete(key);
    this.map.set(key, now);
    this.lastSentAt = now;
    while (this.map.size > this.max) {
      const oldest = this.map.keys().next().value;
      this.map.delete(oldest);
    }
  }

  isEcho(body, scope = '') {
    const key = this.key(body, scope);
    if (!key) return false;
    this.prune();
    return this.map.has(key);
  }

  size() {
    this.prune();
    return this.map.size;
  }

  elapsedSinceLastSent() {
    return this.lastSentAt ? this.now() - this.lastSentAt : -1;
  }

  reset() {
    this.map.clear();
    this.lastSentAt = 0;
  }
}

module.exports = { EchoTracker };
