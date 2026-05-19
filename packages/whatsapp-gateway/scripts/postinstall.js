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
// Baileys 6.7.21 (and recent 6.x) issues `Promise.all([fetchProps,
// fetchBlocklist, fetchPrivacySettings])` during the initial post-auth
// handshake (`executeInitQueries` in
// `node_modules/@whiskeysockets/baileys/lib/Socket/chats.js`). WhatsApp's
// server protocol drifted recently so `fetchProps` returns a 408 Request
// Time-Out after 60s; with `Promise.all` the timeout reject takes the whole
// init-queries flow down, which keeps the gateway in a reconnect loop and
// silently swallows inbound messages — the user-visible symptom is "Ambrogio
// doesn't reply on WhatsApp anymore" with no error surfaced.
//
// Patch swaps `Promise.all` for `Promise.allSettled`: the gateway tolerates
// the `fetchProps` timeout (chat list / props are not required for
// receive/send), init queries complete, and message handling resumes.
// Idempotent: silently skips when the patched call site already contains
// `allSettled`. Runs on every `npm install` so a reinstall (Docker image
// rebuild, lpk recreate) does not regress the gateway back to the broken
// `Promise.all` form.
//
// This is a stop-gap until the upstream `whatsapp-gateway` migrates to
// Baileys 7.x (where `fetchProps` is rewritten and the timeout no longer
// blocks the init flow). The patch is a no-op against 7.x because the
// `Promise.all(... fetchProps()...)` call site is gone.
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
  if (
    src.includes(
      'Promise.allSettled([fetchProps(), fetchBlocklist(), fetchPrivacySettings()])',
    )
  ) {
    return; // already patched
  }
  if (
    !src.includes(
      'Promise.all([fetchProps(), fetchBlocklist(), fetchPrivacySettings()])',
    )
  ) {
    return; // Baileys version doesn't expose this call shape (e.g. 7.x)
  }
  const patched = src.replace(
    'Promise.all([fetchProps(), fetchBlocklist(), fetchPrivacySettings()])',
    'Promise.allSettled([fetchProps(), fetchBlocklist(), fetchPrivacySettings()])',
  );
  fs.writeFileSync(chatsJs, patched, 'utf8');
  console.log(
    '[postinstall] Patched Baileys executeInitQueries: Promise.all -> Promise.allSettled',
  );
}

try {
  patchBaileysInitQueries();
} catch (err) {
  console.warn('[postinstall] Baileys patch skipped:', err.message);
}

// Detect Termux/Android: Node on Termux reports os.platform() as 'android',
// or the kernel version contains 'android', or the Termux prefix exists.
function isTermux() {
  if (os.platform() === 'android') return true;
  if (os.release().toLowerCase().includes('android')) return true;
  if (fs.existsSync('/data/data/com.termux')) return true;
  return false;
}

if (!isTermux()) {
  process.exit(0);
}

// Check if better-sqlite3 native addon already works
const betterSqlite3Dir = path.join(__dirname, '..', 'node_modules', 'better-sqlite3');
if (!fs.existsSync(betterSqlite3Dir)) {
  // Not installed yet (shouldn't happen in postinstall, but bail out gracefully)
  process.exit(0);
}

try {
  require('better-sqlite3');
  // Native addon loads fine — no patching needed
  process.exit(0);
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
    process.exit(0);
  }
}

if (!fs.existsSync(gypiPath)) {
  console.warn('[postinstall] common.gypi not found at', gypiPath);
  console.warn('[postinstall] Skipping native addon rebuild. You may need to patch common.gypi manually.');
  process.exit(0);
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
