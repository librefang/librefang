'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

test('PM2 config creates the log directory and accepts bounded overrides', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'librefang-pm2-'));
  const logDir = path.join(root, 'nested', 'logs');
  process.env.WA_GATEWAY_CWD = root;
  process.env.WA_GATEWAY_LOG_DIR = logDir;
  process.env.WA_GATEWAY_MAX_RESTARTS = '12';
  process.env.WA_GATEWAY_MAX_MEMORY = '768M';
  process.env.WA_GATEWAY_KILL_TIMEOUT_MS = '7000';
  const config = require('../ecosystem.config.cjs');
  const app = config.apps[0];
  assert.equal(fs.statSync(logDir).isDirectory(), true);
  assert.equal(app.max_restarts, 12);
  assert.equal(app.max_memory_restart, '768M');
  assert.equal(app.kill_timeout, 7000);
});
