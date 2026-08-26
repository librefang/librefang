'use strict';

// Tests for the Baileys `executeInitQueries` non-blocking patch shipped by
// `scripts/postinstall.js`. Uses a temp-dir fixture that mimics the
// `node_modules/@whiskeysockets/baileys/lib/Socket/chats.js` layout so we
// don't have to install Baileys in CI.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { execFileSync } = require('node:child_process');

const SCRIPT = path.join(__dirname, '..', 'scripts', 'postinstall.js');

function mkFixture(chatsJsContents, version = '6.7.22') {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'librefang-postinstall-'));
  // Mirror the layout the patcher expects:
  //   <fixture>/scripts/postinstall.js     (copy of the real script — copy,
  //                                         not symlink, so __dirname inside
  //                                         the script resolves to the
  //                                         fixture's scripts/ dir)
  //   <fixture>/node_modules/@whiskeysockets/baileys/lib/Socket/chats.js
  const scriptsDir = path.join(root, 'scripts');
  fs.mkdirSync(scriptsDir, { recursive: true });
  fs.copyFileSync(SCRIPT, path.join(scriptsDir, 'postinstall.js'));
  const chatsDir = path.join(
    root,
    'node_modules',
    '@whiskeysockets',
    'baileys',
    'lib',
    'Socket',
  );
  fs.mkdirSync(chatsDir, { recursive: true });
  const chatsJs = path.join(chatsDir, 'chats.js');
  fs.writeFileSync(chatsJs, chatsJsContents, 'utf8');
  fs.writeFileSync(
    path.join(root, 'node_modules', '@whiskeysockets', 'baileys', 'package.json'),
    JSON.stringify({ name: '@whiskeysockets/baileys', version }),
    'utf8',
  );
  return { root, chatsJs };
}

function runPostinstall(fixtureRoot) {
  return execFileSync(process.execPath, [path.join(fixtureRoot, 'scripts', 'postinstall.js')], {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  });
}

const VANILLA_INIT_QUERIES = `
    const executeInitQueries = async () => {
        await Promise.all([fetchProps(), fetchBlocklist(), fetchPrivacySettings()]);
    };
`;

test('patches vanilla Baileys 6.7.x: Promise.all -> allSettled with per-promise catches', () => {
  const { root, chatsJs } = mkFixture(VANILLA_INIT_QUERIES);
  const out = runPostinstall(root);
  const after = fs.readFileSync(chatsJs, 'utf8');
  assert.ok(after.includes('Promise.allSettled'), 'allSettled present');
  assert.ok(after.includes('[librefang-baileys-patch]'), 'per-promise log marker present');
  assert.ok(
    after.includes("fetchProps().catch"),
    'fetchProps wrapped with individual .catch',
  );
  assert.ok(
    after.includes("fetchBlocklist().catch"),
    'fetchBlocklist wrapped with individual .catch',
  );
  assert.ok(
    after.includes("fetchPrivacySettings().catch"),
    'fetchPrivacySettings wrapped with individual .catch',
  );
  assert.ok(
    !after.includes('Promise.all([fetchProps()'),
    'original Promise.all call site is gone',
  );
  assert.match(out, /Patched Baileys executeInitQueries/);
});

test('idempotent: second run is a no-op against the new-shape patch', () => {
  const { root, chatsJs } = mkFixture(VANILLA_INIT_QUERIES);
  runPostinstall(root);
  const after1 = fs.readFileSync(chatsJs, 'utf8');
  const out2 = runPostinstall(root);
  const after2 = fs.readFileSync(chatsJs, 'utf8');
  assert.equal(after1, after2, 'file unchanged on second run');
  assert.doesNotMatch(out2, /Patched Baileys executeInitQueries/);
});

test('upgrades older simple-patched form (allSettled-only) to per-promise catches', () => {
  const SIMPLE_PATCHED = `
    const executeInitQueries = async () => {
        await Promise.allSettled([fetchProps(), fetchBlocklist(), fetchPrivacySettings()]);
    };
  `;
  const { root, chatsJs } = mkFixture(SIMPLE_PATCHED);
  runPostinstall(root);
  const after = fs.readFileSync(chatsJs, 'utf8');
  assert.ok(after.includes('[librefang-baileys-patch]'), 'log marker added on upgrade');
  assert.ok(after.includes('fetchProps().catch'), 'per-promise catch added on upgrade');
});

test('no-op on Baileys 7.x (call site shape gone) — exits cleanly without modifying the file', () => {
  const BAILEYS_7X = `
    const executeInitQueries = async () => {
        // Baileys 7.x rewrote this; the literal call site is gone.
        await Promise.allSettled([
            fetchProps().catch(() => {}),
        ]);
    };
  `;
  const { root, chatsJs } = mkFixture(BAILEYS_7X, '7.0.0');
  const before = fs.readFileSync(chatsJs, 'utf8');
  runPostinstall(root);
  const after = fs.readFileSync(chatsJs, 'utf8');
  assert.equal(before, after, 'file unchanged when Baileys shape does not match');
});

test('fails loudly when Baileys is missing the expected call site (e.g. major rewrite)', () => {
  const REWRITTEN = `
    const executeInitQueries = async () => {
        // entirely refactored
        await runInitQueries({ props: true, blocklist: true });
    };
  `;
  const { root, chatsJs } = mkFixture(REWRITTEN);
  const before = fs.readFileSync(chatsJs, 'utf8');
  assert.throws(
    () => runPostinstall(root),
    (error) => {
      assert.match(error.stderr, /unrecognized executeInitQueries shape in Baileys 6\.7\.22/);
      return true;
    },
  );
  const after = fs.readFileSync(chatsJs, 'utf8');
  assert.equal(before, after, 'unrecognized Baileys shape is not modified');
});

test('post-write verification requires every per-query marker', () => {
  const script = require('../scripts/postinstall.js');
  assert.equal(typeof script.patchBaileysInitQueries, 'function');
  assert.equal(typeof script.isTermux, 'function');
  assert.ok(script.BAILEYS_INIT_QUERIES_NEEDLE.includes('Promise.all(['));
  assert.ok(script.BAILEYS_INIT_QUERIES_REPLACEMENT.includes('Promise.allSettled'));
  assert.ok(script.BAILEYS_INIT_QUERIES_REPLACEMENT.includes('[librefang-baileys-patch]'));
  assert.equal(script.hasCompleteBaileysPatch(script.BAILEYS_INIT_QUERIES_REPLACEMENT), true);
  for (const marker of [
    'fetchProps rejected',
    'fetchBlocklist rejected',
    'fetchPrivacySettings rejected',
  ]) {
    assert.equal(
      script.hasCompleteBaileysPatch(script.BAILEYS_INIT_QUERIES_REPLACEMENT.replaceAll(marker, 'missing')),
      false,
      marker,
    );
  }
});

test('patchAndroidNdkCflags removes only the Termux NDK include', () => {
  const { patchAndroidNdkCflags } = require('../scripts/postinstall.js');
  const input = "'cflags': [ '-fPIC', '-I<(android_ndk_path)/sources/android/cpufeatures' ],";
  assert.equal(patchAndroidNdkCflags(input), "'cflags': [ '-fPIC' ],");
});

test('patchAndroidNdkCflags rejects unknown NDK cflags shapes', () => {
  const { patchAndroidNdkCflags } = require('../scripts/postinstall.js');
  assert.throws(
    () => patchAndroidNdkCflags("'cflags': [ '-Wall', '-I<(android_ndk_path)/include' ]"),
    /cflags shape is unrecognized/,
  );
});
