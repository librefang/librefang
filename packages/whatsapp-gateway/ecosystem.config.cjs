// Default PM2 ecosystem for the WhatsApp gateway.
//
// Paths are relative to the package directory so the file works out of the
// box for anyone who runs `pm2 start ecosystem.config.cjs` after cloning.
// Operators who want to run the gateway out of a dedicated data volume can
// override `cwd` / log paths via env vars:
//
//   WA_GATEWAY_CWD=/data/whatsapp-gateway pm2 start ecosystem.config.cjs
//
// Deployment-specific values (default agent, allowed senders, ...) are
// read by index.js from LIBREFANG_* env vars at runtime and should be set
// in the deployment environment, not committed here.
const path = require('node:path');

const cwd = process.env.WA_GATEWAY_CWD || __dirname;
const logDir = process.env.WA_GATEWAY_LOG_DIR || path.join(cwd, 'logs');

module.exports = {
  apps: [
    {
      name: 'whatsapp-gateway',
      script: 'index.js',
      cwd,
      watch: false,
      autorestart: true,
      max_restarts: 5,
      min_uptime: '30s',
      restart_delay: 3000,
      max_memory_restart: '256M',
      exp_backoff_restart_delay: 1000,
      error_file: path.join(logDir, 'pm2-error.log'),
      out_file: path.join(logDir, 'pm2-out.log'),
      merge_logs: true,
      time: true,
      env: {
        NODE_ENV: 'production',
      },
    },
  ],
};
