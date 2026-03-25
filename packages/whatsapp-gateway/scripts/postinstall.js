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
