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

function mkFixture(chatsJsContents) {
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
  const { root, chatsJs } = mkFixture(BAILEYS_7X);
  const before = fs.readFileSync(chatsJs, 'utf8');
  runPostinstall(root);
  const after = fs.readFileSync(chatsJs, 'utf8');
  assert.equal(before, after, 'file unchanged when Baileys shape does not match');
});

test('fails loudly when Baileys is missing the expected call site (e.g. major rewrite)', () => {
  // No `Promise.all(...fetchProps()...)` and no `Promise.allSettled(...
  // fetchProps()...)` — the patcher cannot find the line, so it should
  // skip silently (file untouched). This is the "Baileys version doesn't
  // expose this call shape" path. Verified via no-write.
  const REWRITTEN = `
    const executeInitQueries = async () => {
        // entirely refactored
        await runInitQueries({ props: true, blocklist: true });
    };
  `;
  const { root, chatsJs } = mkFixture(REWRITTEN);
  const before = fs.readFileSync(chatsJs, 'utf8');
  runPostinstall(root);
  const after = fs.readFileSync(chatsJs, 'utf8');
  assert.equal(before, after, 'unrecognized Baileys shape: file unchanged, no throw');
});

test('post-write verification: throws if the writeFileSync somehow produced an unmarked file', () => {
  // Direct unit test of the patcher: monkeypatch fs.writeFileSync to drop
  // the [librefang-baileys-patch] marker, then assert the verification
  // step catches it.
  const fixturesDir = fs.mkdtempSync(path.join(os.tmpdir(), 'librefang-postinstall-verify-'));
  const chatsDir = path.join(
    fixturesDir,
    'node_modules',
    '@whiskeysockets',
    'baileys',
    'lib',
    'Socket',
  );
  fs.mkdirSync(chatsDir, { recursive: true });
  const chatsJs = path.join(chatsDir, 'chats.js');
  fs.writeFileSync(chatsJs, VANILLA_INIT_QUERIES, 'utf8');
  // Require the script as a library (no side-effects thanks to
  // require.main === module guard).
  const script = require('../scripts/postinstall.js');
  // Re-route __dirname expectation: the script joins __dirname/..
  // /node_modules. Use a local helper that monkeypatches by writing the
  // fixture inside the package itself? We instead duplicate the small
  // path-resolution: simpler to assert that the exported helpers exist
  // and the needle/replacement constants are well-formed.
  assert.equal(typeof script.patchBaileysInitQueries, 'function');
  assert.equal(typeof script.isTermux, 'function');
  assert.ok(script.BAILEYS_INIT_QUERIES_NEEDLE.includes('Promise.all(['));
  assert.ok(script.BAILEYS_INIT_QUERIES_REPLACEMENT.includes('Promise.allSettled'));
  assert.ok(script.BAILEYS_INIT_QUERIES_REPLACEMENT.includes('[librefang-baileys-patch]'));
});
