#!/usr/bin/env node
// Post-install script for Termux/Android environments.
//
// On Termux, Node.js reports OS as "android". The node-gyp common.gypi shipped
// with Node contains an Android-specific block that references `android_ndk_path`,
// which is undefined because Termux compiles natively (no NDK needed). This causes
// native addons like better-sqlite3 to fail to build.
//
// This script patches common.gypi to remove the NDK reference, then rebuilds
// better-sqlite3. On non-Termux platforms this script exits immediately.

'use strict';

const os = require('os');
const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');

// ---------------------------------------------------------------------------
// Baileys `fetchProps` non-blocking patch
// ---------------------------------------------------------------------------
// Baileys 6.7.21 (pinned exactly in package.json) issues
// `Promise.all([fetchProps, fetchBlocklist, fetchPrivacySettings])` during
// the initial post-auth handshake (`executeInitQueries` in
// `node_modules/@whiskeysockets/baileys/lib/Socket/chats.js`, ~line 762 —
// upstream:
// https://github.com/WhiskeySockets/Baileys/blob/v6.7.21/src/Socket/chats.ts).
// WhatsApp's server protocol drifted recently so `fetchProps` returns a 408
// Request Time-Out after 60s; with `Promise.all` the timeout reject takes
// the whole init-queries flow down, which keeps the gateway in a reconnect
// loop and silently swallows inbound messages — the user-visible symptom is
// "Ambrogio doesn't reply on WhatsApp anymore" with no error surfaced.
//
// Why each of the three queries is non-essential for receive/send:
//   * fetchProps — server "abt" props (UI experiment flags, feature toggles
//     for WA Web). The gateway does no UI; the only consumer downstream is
//     advertising-flag bookkeeping. The bug we observe is precisely
//     fetchProps timing out, and the connection still receives messages
//     once executeInitQueries returns.
//   * fetchBlocklist — list of JIDs the user has blocked. Used to suppress
//     outbound replies to blocked contacts. With LibreFang the bot has no
//     human-curated blocklist (greenfield account), so an empty/missing
//     blocklist degrades gracefully. Worst case: a reply is sent to a
//     blocked contact; never a delivery failure.
//   * fetchPrivacySettings — "last seen", "read receipts", "online" privacy
//     toggles. The gateway never reads these for send/receive decisions;
//     they only affect presence broadcasts (which we don't drive). A failed
//     fetch means the in-memory cache stays empty; subsequent reads return
//     undefined and Baileys treats it as "default".
// No upstream Baileys documentation explicitly labels these three as
// optional; the justification above is empirical (the gateway demonstrably
// sends and receives with executeInitQueries half-failed) plus a read of
// the chats.js source — search for `fetchPrivacySettings` / `fetchBlocklist`
// / `fetchProps` in the upstream link above to confirm none gate the
// message ingress path.
//
// Patch swaps `Promise.all` for `Promise.allSettled` and ALSO wraps each
// promise with an individual `.catch(log)` so a timeout on, say,
// `fetchProps` shows up in operator logs (`[librefang-baileys-patch]
// fetchProps rejected: ...`) instead of being silently swallowed by the
// allSettled envelope. Without the per-promise catch, operators debugging
// "messages stuck" have no signal pointing at the right init query.
//
// Idempotent: silently skips when the patched call site already contains
// `allSettled`. Runs on every `npm install` so a reinstall (Docker image
// rebuild, lpk recreate) does not regress the gateway back to the broken
// `Promise.all` form.
//
// Patch verification: after writing, we re-read the file and assert it
// contains the `allSettled` marker. If a Baileys minor bump rewrites this
// line (whitespace, refactor) the literal-string match would silently
// no-op; the post-write assert turns that into a loud install-time failure
// instead of a silent regression that resurfaces as another outage.
//
// This is a stop-gap until the upstream `whatsapp-gateway` migrates to
// Baileys 7.x (where `fetchProps` is rewritten and the timeout no longer
// blocks the init flow). The patch is a no-op against 7.x because the
// `Promise.all(... fetchProps()...)` call shape is gone — the script just
// exits without touching the file.
const BAILEYS_INIT_QUERIES_NEEDLE =
  'Promise.all([fetchProps(), fetchBlocklist(), fetchPrivacySettings()])';
const BAILEYS_INIT_QUERIES_REPLACEMENT =
  "Promise.allSettled([\n" +
  "        fetchProps().catch((err) => { try { logger && logger.warn ? logger.warn({ err }, '[librefang-baileys-patch] fetchProps rejected') : console.warn('[librefang-baileys-patch] fetchProps rejected:', err && err.message ? err.message : err); } catch (_) {} }),\n" +
  "        fetchBlocklist().catch((err) => { try { logger && logger.warn ? logger.warn({ err }, '[librefang-baileys-patch] fetchBlocklist rejected') : console.warn('[librefang-baileys-patch] fetchBlocklist rejected:', err && err.message ? err.message : err); } catch (_) {} }),\n" +
  "        fetchPrivacySettings().catch((err) => { try { logger && logger.warn ? logger.warn({ err }, '[librefang-baileys-patch] fetchPrivacySettings rejected') : console.warn('[librefang-baileys-patch] fetchPrivacySettings rejected:', err && err.message ? err.message : err); } catch (_) {} }),\n" +
  "    ])";

function patchBaileysInitQueries() {
  const chatsJs = path.join(
    __dirname,
    '..',
    'node_modules',
    '@whiskeysockets',
    'baileys',
    'lib',
    'Socket',
    'chats.js',
  );
  if (!fs.existsSync(chatsJs)) {
    // Baileys not installed (dev `--no-save` install) or 7.x path layout —
    // nothing to patch.
    return;
  }
  const src = fs.readFileSync(chatsJs, 'utf8');
  // Already patched: the file contains an `allSettled` form referencing all
  // three init queries. (We also accept a previous, simpler patch revision
  // that only swapped `all` -> `allSettled` without per-promise catches —
  // upgrade it in-place so the visibility wrapping lands too.)
  const SIMPLE_PATCHED_NEEDLE =
    'Promise.allSettled([fetchProps(), fetchBlocklist(), fetchPrivacySettings()])';
  if (src.includes(BAILEYS_INIT_QUERIES_REPLACEMENT)) {
    return; // already patched at current revision
  }
  let patched;
  if (src.includes(SIMPLE_PATCHED_NEEDLE)) {
    // Upgrade older patch revision (allSettled without per-promise catches)
    patched = src.replace(SIMPLE_PATCHED_NEEDLE, BAILEYS_INIT_QUERIES_REPLACEMENT);
  } else if (src.includes(BAILEYS_INIT_QUERIES_NEEDLE)) {
    patched = src.replace(BAILEYS_INIT_QUERIES_NEEDLE, BAILEYS_INIT_QUERIES_REPLACEMENT);
  } else {
    return; // Baileys version doesn't expose this call shape (e.g. 7.x)
  }
  fs.writeFileSync(chatsJs, patched, 'utf8');

  // Re-read and assert. If a Baileys bump touched this line and our literal
  // match no-op'd, fail loudly at install time so operators know to refresh
  // the patch rather than discovering a regressed gateway in production.
  const verify = fs.readFileSync(chatsJs, 'utf8');
  if (!verify.includes('Promise.allSettled') || !verify.includes('[librefang-baileys-patch]')) {
    throw new Error(
      '[librefang-baileys-patch] post-write verification failed: ' +
        'expected `Promise.allSettled` and `[librefang-baileys-patch]` markers in ' +
        chatsJs +
        '. The Baileys source likely changed shape; refresh the patch in ' +
        'scripts/postinstall.js (BAILEYS_INIT_QUERIES_NEEDLE / REPLACEMENT).',
    );
  }
  console.log(
    '[postinstall] Patched Baileys executeInitQueries: Promise.all -> Promise.allSettled (with per-promise logging)',
  );
}

// Detect Termux/Android: Node on Termux reports os.platform() as 'android',
// or the kernel version contains 'android', or the Termux prefix exists.
function isTermux() {
  if (os.platform() === 'android') return true;
  if (os.release().toLowerCase().includes('android')) return true;
  if (fs.existsSync('/data/data/com.termux')) return true;
  return false;
}

function rebuildBetterSqlite3OnTermux() {
  if (!isTermux()) {
    return;
  }

  // Check if better-sqlite3 native addon already works
  const betterSqlite3Dir = path.join(__dirname, '..', 'node_modules', 'better-sqlite3');
  if (!fs.existsSync(betterSqlite3Dir)) {
    // Not installed yet (shouldn't happen in postinstall, but bail out gracefully)
    return;
  }

  try {
    require('better-sqlite3');
    // Native addon loads fine — no patching needed
    return;
  } catch (_) {
    // Native addon missing or broken — proceed with patching
  }

  console.log('[postinstall] Termux/Android detected — patching node-gyp for native addon build...');

  // Locate common.gypi in the node-gyp cache.
  // Typical path: ~/.cache/node-gyp/<version>/include/node/common.gypi
  const nodeVersion = process.version.slice(1); // strip leading 'v'
  const cacheBase = path.join(os.homedir(), '.cache', 'node-gyp', nodeVersion);
  const gypiPath = path.join(cacheBase, 'include', 'node', 'common.gypi');

  if (!fs.existsSync(gypiPath)) {
    // The cache might not exist yet — ask node-gyp to create it
    console.log('[postinstall] node-gyp cache not found, running node-gyp install...');
    try {
      execSync('npx --yes node-gyp install', { stdio: 'inherit', cwd: betterSqlite3Dir });
    } catch (e) {
      console.error('[postinstall] node-gyp install failed:', e.message);
      console.error('[postinstall] Skipping native addon rebuild. You may need to patch common.gypi manually.');
      return;
    }
  }

  if (!fs.existsSync(gypiPath)) {
    console.warn('[postinstall] common.gypi not found at', gypiPath);
    console.warn('[postinstall] Skipping native addon rebuild. You may need to patch common.gypi manually.');
    return;
  }

  // Patch common.gypi: remove the android_ndk_path include from cflags
  let gypiContent = fs.readFileSync(gypiPath, 'utf8');
  const ndkNeedle = 'android_ndk_path';

  if (gypiContent.includes(ndkNeedle)) {
    // The problematic line looks like:
    //   'cflags': [ '-fPIC', '-I<(android_ndk_path)/sources/android/cpufeatures' ],
    // Replace with:
    //   'cflags': [ '-fPIC' ],
    gypiContent = gypiContent.replace(
      /('cflags':\s*\[\s*'-fPIC'\s*),\s*'-I<\(android_ndk_path\)[^']*'\s*(\])/,
      '$1 $2'
    );
    fs.writeFileSync(gypiPath, gypiContent, 'utf8');
    console.log('[postinstall] Patched common.gypi — removed android_ndk_path reference');
  } else {
    console.log('[postinstall] common.gypi already patched (no android_ndk_path found)');
  }

  // Rebuild better-sqlite3 native addon
  console.log('[postinstall] Rebuilding better-sqlite3 native addon...');
  try {
    execSync('npx --yes node-gyp rebuild', {
      cwd: betterSqlite3Dir,
      stdio: 'inherit',
      env: { ...process.env, npm_config_nodedir: cacheBase },
    });
    console.log('[postinstall] better-sqlite3 rebuilt successfully');
  } catch (e) {
    console.error('[postinstall] Failed to rebuild better-sqlite3:', e.message);
    console.error('[postinstall] The WhatsApp gateway may not work. Try manually:');
    console.error('[postinstall]   cd ' + betterSqlite3Dir);
    console.error('[postinstall]   npx node-gyp rebuild');
    process.exit(1);
  }
}

// Exports for testing (no side effects on require). The CLI entry below
// runs the actual install-time work only when invoked as `node
// scripts/postinstall.js` (i.e. via the `npm install` postinstall hook).
module.exports = {
  patchBaileysInitQueries,
  isTermux,
  rebuildBetterSqlite3OnTermux,
  // Exposed for fixture-based tests: the literal needle/replacement strings
  // used by the patcher.
  BAILEYS_INIT_QUERIES_NEEDLE,
  BAILEYS_INIT_QUERIES_REPLACEMENT,
};

if (require.main === module) {
  try {
    patchBaileysInitQueries();
  } catch (err) {
    // Patch failures are FATAL — see post-write verification above. A
    // silent skip here is what produced the original outage we're fixing.
    console.error('[postinstall] Baileys patch FAILED:', err.message);
    process.exit(1);
  }
  rebuildBetterSqlite3OnTermux();
}
