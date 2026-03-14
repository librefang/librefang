const FLY_API = 'https://api.machines.dev/v1';
const DOCKER_IMAGE = 'ghcr.io/librefang/librefang:latest';
const REGION = 'nrt';
const FREE_MODEL = 'openrouter/stepfun/step-3.5-flash:free';

// Default config.toml for deployed instances (uses OpenRouter free model)
const DEFAULT_CONFIG = `
api_listen = "0.0.0.0:4545"
log_level = "info"

[default_model]
provider = "openrouter"
model = "${FREE_MODEL}"
api_key_env = "OPENROUTER_API_KEY"
`.trim();

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    if (url.pathname === '/api/deploy' && request.method === 'POST') {
      return handleDeploy(request, env);
    }

    return new Response(HTML, {
      headers: { 'Content-Type': 'text/html; charset=utf-8' },
    });
  },
};

async function handleDeploy(request, env) {
  try {
    const body = await request.json();
    const { token } = body;

    if (!token) {
      return json({ error: 'API Token is required' }, 400);
    }

    const headers = {
      Authorization: `Bearer ${token}`,
      'Content-Type': 'application/json',
    };

    // 1. Verify token
    const orgsRes = await fetch(`${FLY_API}/apps`, { headers });
    if (!orgsRes.ok) {
      return json({ error: 'Invalid API Token. Please check and try again.' }, 401);
    }

    // 2. Create app
    const appName = `librefang-${randomHex(6)}`;
    const appRes = await fetch(`${FLY_API}/apps`, {
      method: 'POST',
      headers,
      body: JSON.stringify({ app_name: appName, org_slug: 'personal' }),
    });
    if (!appRes.ok) {
      const err = await appRes.text();
      return json({ error: `Failed to create app: ${err}` }, 500);
    }

    // 3. Allocate shared IPv4 + IPv6 (needed for public HTTPS)
    const flyGraphQL = 'https://api.fly.io/graphql';
    const gqlHeaders = { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' };

    await fetch(flyGraphQL, {
      method: 'POST',
      headers: gqlHeaders,
      body: JSON.stringify({
        query: `mutation { allocateIPAddress(input: { appId: "${appName}", type: shared_v4 }) { ipAddress { address type } } }`,
      }),
    });
    await fetch(flyGraphQL, {
      method: 'POST',
      headers: gqlHeaders,
      body: JSON.stringify({
        query: `mutation { allocateIPAddress(input: { appId: "${appName}", type: v6 }) { ipAddress { address type } } }`,
      }),
    });

    // 4. Create volume
    const volRes = await fetch(`${FLY_API}/apps/${appName}/volumes`, {
      method: 'POST',
      headers,
      body: JSON.stringify({ name: 'librefang_data', region: REGION, size_gb: 1 }),
    });
    if (!volRes.ok) {
      const err = await volRes.text();
      return json({ error: `Failed to create volume: ${err}` }, 500);
    }

    // 5. Build env — inject built-in OpenRouter key + default config
    const builtinKey = env.OPENROUTER_API_KEY || '';
    const env_vars = {
      LIBREFANG_HOME: '/data',
      OPENROUTER_API_KEY: builtinKey,
      LIBREFANG_DEFAULT_CONFIG: DEFAULT_CONFIG,
    };

    // 6. Create machine
    const machineRes = await fetch(`${FLY_API}/apps/${appName}/machines`, {
      method: 'POST',
      headers,
      body: JSON.stringify({
        region: REGION,
        config: {
          image: DOCKER_IMAGE,
          env: env_vars,
          guest: { cpu_kind: 'shared', cpus: 1, memory_mb: 256 },
          services: [
            {
              ports: [
                { port: 443, handlers: ['tls', 'http'] },
                { port: 80, handlers: ['http'] },
              ],
              protocol: 'tcp',
              internal_port: 4545,
              force_instance_key: null,
            },
          ],
          mounts: [{ volume: 'librefang_data', path: '/data' }],
          auto_destroy: false,
        },
      }),
    });

    if (!machineRes.ok) {
      const err = await machineRes.text();
      return json({ error: `Failed to create machine: ${err}` }, 500);
    }

    const machine = await machineRes.json();
    const appUrl = `https://${appName}.fly.dev`;

    return json({
      success: true,
      appName,
      url: appUrl,
      dashboardUrl: `https://fly.io/apps/${appName}`,
      machineId: machine.id,
      region: REGION,
    });
  } catch (err) {
    return json({ error: err.message || 'Unexpected error' }, 500);
  }
}

function randomHex(len) {
  const arr = new Uint8Array(len);
  crypto.getRandomValues(arr);
  return Array.from(arr, (b) => b.toString(16).padStart(2, '0')).join('');
}

function json(data, status = 200) {
  return new Response(JSON.stringify(data), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

const HTML = `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Deploy LibreFang</title>
  <link rel="icon" href="https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png">
  <style>
    :root {
      --bg: #0a0a0f;
      --surface: #12121a;
      --surface2: #1a1a26;
      --border: #1e1e2e;
      --text: #e4e4ef;
      --dim: #8888a0;
      --accent: #8b5cf6;
      --accent-hover: #7c3aed;
      --green: #34d399;
      --red: #f87171;
      --orange: #f59e0b;
      --blue: #60a5fa;
    }
    * { margin: 0; padding: 0; box-sizing: border-box; }
    body {
      font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', system-ui, sans-serif;
      background: var(--bg);
      color: var(--text);
      min-height: 100vh;
      display: flex;
      justify-content: center;
      padding: 40px 16px;
    }
    .container { max-width: 680px; width: 100%; }
    .header { text-align: center; margin-bottom: 36px; }
    .logo { width: 64px; height: 64px; border-radius: 16px; margin-bottom: 16px; }
    h1 {
      font-size: 1.8rem;
      background: linear-gradient(135deg, #e4e4ef, #8b5cf6);
      -webkit-background-clip: text;
      -webkit-text-fill-color: transparent;
      margin-bottom: 8px;
    }
    .subtitle { color: var(--dim); font-size: 0.95rem; }
    .badge-row {
      display: flex; justify-content: center; gap: 8px;
      margin-top: 16px; flex-wrap: wrap;
    }
    .badge {
      display: inline-flex; align-items: center; gap: 6px;
      padding: 6px 12px; border-radius: 20px;
      font-size: 0.8rem; font-weight: 500;
      border: 1px solid var(--border); background: var(--surface);
    }
    .dot { width: 8px; height: 8px; border-radius: 50%; }
    .dot-green { background: var(--green); }
    .dot-purple { background: var(--accent); }
    .dot-orange { background: var(--orange); }

    /* Platform grid */
    .platform-grid {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 14px;
      margin-bottom: 16px;
    }
    .platform-card {
      background: var(--surface);
      border: 1px solid var(--border);
      border-radius: 14px;
      padding: 22px;
      cursor: pointer;
      transition: all 0.2s;
      position: relative;
      text-decoration: none;
      color: var(--text);
      display: block;
      min-width: 0;
    }
    .platform-card:hover {
      border-color: var(--accent);
      transform: translateY(-2px);
      box-shadow: 0 4px 20px rgba(139, 92, 246, 0.15);
    }
    .platform-card.accent-purple:hover { border-color: var(--accent); box-shadow: 0 4px 20px rgba(139, 92, 246, 0.15); }
    .platform-card.accent-green:hover { border-color: var(--green); box-shadow: 0 4px 20px rgba(52, 211, 153, 0.15); }
    .platform-card.accent-blue:hover { border-color: var(--blue); box-shadow: 0 4px 20px rgba(96, 165, 250, 0.15); }
    .platform-icon { font-size: 1.6rem; margin-bottom: 10px; }
    .platform-name { font-weight: 600; font-size: 1rem; margin-bottom: 6px; }
    .platform-desc { color: var(--dim); font-size: 0.83rem; line-height: 1.4; }
    .platform-badge {
      position: absolute;
      top: 12px;
      right: 12px;
      font-size: 0.7rem;
      font-weight: 600;
      padding: 3px 8px;
      border-radius: 10px;
      text-transform: uppercase;
      letter-spacing: 0.03em;
    }
    .badge-recommended { background: rgba(139, 92, 246, 0.2); color: var(--accent); }
    .badge-easiest { background: rgba(52, 211, 153, 0.2); color: var(--green); }
    .badge-terraform { background: rgba(96, 165, 250, 0.2); color: var(--blue); }
    .platform-warning {
      font-size: 0.75rem;
      color: var(--orange);
      margin-top: 6px;
      line-height: 1.3;
    }
    .platform-cmd {
      font-size: 0.78rem;
      color: var(--green);
      background: var(--bg);
      padding: 4px 6px 4px 8px;
      border-radius: 4px;
      margin-top: 6px;
      font-family: monospace;
      display: inline-flex;
      align-items: center;
      gap: 6px;
      max-width: 100%;
      overflow: hidden;
    }
    .platform-cmd code { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .copy-btn {
      background: var(--surface2);
      border: 1px solid var(--border);
      color: var(--dim);
      border-radius: 4px;
      padding: 2px 6px;
      font-size: 0.7rem;
      cursor: pointer;
      flex-shrink: 0;
      transition: all 0.15s;
    }
    .copy-btn:hover { color: var(--green); border-color: var(--green); }

    /* Home button */
    .home-btn {
      display: inline-flex;
      align-items: center;
      gap: 4px;
      color: var(--dim);
      text-decoration: none;
      font-size: 0.85rem;
      margin-bottom: 20px;
      transition: color 0.15s;
    }
    .home-btn:hover { color: var(--accent); }

    /* Back button */
    .back-btn {
      background: none;
      border: 1px solid var(--border);
      color: var(--dim);
      padding: 8px 16px;
      border-radius: 8px;
      cursor: pointer;
      font-size: 0.85rem;
      margin-bottom: 16px;
      transition: all 0.15s;
    }
    .back-btn:hover { border-color: var(--accent); color: var(--text); }

    /* Existing card / form styles */
    .card {
      background: var(--surface);
      border: 1px solid var(--border);
      border-radius: 14px;
      padding: 28px;
      margin-bottom: 16px;
    }
    .step { display: flex; align-items: flex-start; gap: 12px; margin-bottom: 20px; }
    .step:last-child { margin-bottom: 0; }
    .step-num {
      width: 28px; height: 28px; border-radius: 50%;
      background: rgba(139,92,246,0.15); border: 1px solid var(--accent);
      display: flex; align-items: center; justify-content: center;
      font-size: 0.8rem; font-weight: 600; color: var(--accent);
      flex-shrink: 0; margin-top: 2px;
    }
    .step-content { flex: 1; }
    .step-title { font-weight: 600; margin-bottom: 4px; font-size: 0.95rem; }
    .step-desc { color: var(--dim); font-size: 0.85rem; line-height: 1.5; }
    .step-desc a { color: var(--accent); text-decoration: none; }
    .step-desc a:hover { text-decoration: underline; }
    label { display: block; font-size: 0.85rem; font-weight: 500; margin-bottom: 6px; color: var(--dim); }
    input {
      width: 100%; padding: 10px 14px; border-radius: 8px;
      border: 1px solid var(--border); background: var(--bg);
      color: var(--text); font-size: 0.9rem; outline: none;
      transition: border-color 0.15s;
    }
    input:focus { border-color: var(--accent); }
    input::placeholder { color: #555; }
    .field { margin-bottom: 14px; }
    .btn {
      width: 100%; padding: 14px; border: none; border-radius: 10px;
      background: var(--accent); color: white; font-size: 1rem;
      font-weight: 600; cursor: pointer; transition: all 0.15s;
      margin-top: 8px;
    }
    .btn:hover { background: var(--accent-hover); }
    .btn:disabled { opacity: 0.5; cursor: not-allowed; }
    .btn.deploying {
      background: var(--surface2);
      border: 1px solid var(--border);
      color: var(--dim);
    }
    .free-note {
      background: rgba(52,211,153,0.08);
      border: 1px solid rgba(52,211,153,0.2);
      border-radius: 8px; padding: 12px 16px;
      font-size: 0.85rem; color: var(--green);
      margin-bottom: 16px; line-height: 1.5;
    }
    .result { display: none; }
    .result.show { display: block; }
    .result-success {
      background: rgba(52,211,153,0.08);
      border: 1px solid rgba(52,211,153,0.3);
      border-radius: 12px; padding: 24px; text-align: center;
    }
    .result-success h2 { color: var(--green); font-size: 1.3rem; margin-bottom: 12px; }
    .result-link {
      display: inline-block; padding: 10px 24px; background: var(--green);
      color: #0a0a0f; text-decoration: none; border-radius: 8px;
      font-weight: 600; margin: 8px 4px; font-size: 0.9rem;
    }
    .result-link.secondary {
      background: var(--surface2); color: var(--text);
      border: 1px solid var(--border);
    }
    .result-info { color: var(--dim); font-size: 0.85rem; margin-top: 16px; line-height: 1.6; }
    .result-info code { color: var(--green); background: var(--surface); padding: 2px 6px; border-radius: 4px; font-size: 0.8rem; }
    .error-msg {
      background: rgba(248,113,113,0.1);
      border: 1px solid rgba(248,113,113,0.3);
      border-radius: 8px; padding: 12px 16px;
      color: var(--red); font-size: 0.85rem;
      margin-top: 12px; display: none;
    }
    .error-msg.show { display: block; }
    .progress { display: none; margin-top: 16px; }
    .progress.show { display: block; }
    .progress-step {
      display: flex; align-items: center; gap: 8px;
      font-size: 0.85rem; color: var(--dim); padding: 4px 0;
    }
    .progress-step.active { color: var(--text); }
    .progress-step.done { color: var(--green); }
    .progress-step .icon { width: 18px; text-align: center; }
    .spinner { display: inline-block; width: 14px; height: 14px;
      border: 2px solid var(--border); border-top-color: var(--accent);
      border-radius: 50%; animation: spin 0.6s linear infinite;
    }
    @keyframes spin { to { transform: rotate(360deg); } }
    .footer { text-align: center; padding: 24px; color: var(--dim); font-size: 0.8rem; }
    .footer a { color: var(--accent); text-decoration: none; }
    @media (max-width: 600px) {
      body { padding: 24px 12px; }
      h1 { font-size: 1.4rem; }
      .card { padding: 20px; }
      .platform-grid { grid-template-columns: 1fr; }
    }
  </style>
</head>
<body>
  <div class="container">
    <a href="/" class="home-btn">&larr; deploy.librefang.ai</a>

    <div class="header">
      <img src="https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png" alt="LibreFang" class="logo">
      <h1>Deploy LibreFang</h1>
      <p class="subtitle">Choose your platform</p>
      <div class="badge-row">
        <span class="badge"><span class="dot dot-green"></span>Free LLM included</span>
        <span class="badge"><span class="dot dot-purple"></span>No API key needed</span>
        <span class="badge"><span class="dot dot-orange"></span>1 GB storage</span>
      </div>
    </div>

    <!-- Platform selection grid -->
    <div id="platform-selection">
      <div class="platform-grid">
        <div class="platform-card accent-purple" onclick="showFlyDeploy()">
          <span class="platform-badge badge-recommended">Recommended</span>
          <div class="platform-icon"><svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M17.8 19.2L16 11l3.5-3.5C21 6 21.5 4 21 3c-1-.5-3 0-4.5 1.5L13 8 4.8 6.2c-.5-.1-.9.1-1.1.5l-.3.5c-.2.5-.1 1 .3 1.3L9 12l-2 3H4l-1 1 3 2 2 3 1-1v-3l3-2 3.5 5.3c.3.4.8.5 1.3.3l.5-.3c.4-.2.6-.6.5-1.1z"/></svg></div>
          <div class="platform-name">Fly.io</div>
          <div class="platform-desc">Free forever, persistent storage</div>
        </div>

        <a class="platform-card accent-green" href="https://render.com/deploy?repo=https://github.com/librefang/librefang" target="_blank" rel="noopener">
          <span class="platform-badge badge-easiest">Easiest</span>
          <div class="platform-icon"><svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="2" width="20" height="8" rx="2"/><rect x="2" y="14" width="20" height="8" rx="2"/><circle cx="6" cy="6" r="1" fill="currentColor"/><circle cx="6" cy="18" r="1" fill="currentColor"/></svg></div>
          <div class="platform-name">Render</div>
          <div class="platform-desc">One-click OAuth deploy</div>
          <div class="platform-warning">Free tier: sleeps after 15 min, no persistent storage</div>
        </a>

        <a class="platform-card accent-blue" href="https://railway.com/new/template?template=https://github.com/librefang/librefang" target="_blank" rel="noopener">
          <div class="platform-icon"><svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M4 15s1-1 4-1 5 2 8 2 4-1 4-1V3s-1 1-4 1-5-2-8-2-4 1-4 1z"/><line x1="4" y1="22" x2="4" y2="15"/></svg></div>
          <div class="platform-name">Railway</div>
          <div class="platform-desc">Simple deploy with $5 free credit</div>
        </a>

        <a class="platform-card accent-blue" href="https://github.com/librefang/librefang/tree/main/infra/gcp" target="_blank" rel="noopener">
          <span class="platform-badge badge-terraform">Terraform</span>
          <div class="platform-icon"><svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M18 10h-1.26A8 8 0 1 0 9 20h9a5 5 0 0 0 0-10z"/></svg></div>
          <div class="platform-name">GCP</div>
          <div class="platform-desc">Free forever (e2-micro), 30GB storage</div>
        </a>

        <a class="platform-card accent-blue" href="https://github.com/librefang/librefang/blob/main/docker-compose.yml" target="_blank" rel="noopener">
          <div class="platform-icon"><svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M22 12H2"/><path d="M5.45 5.11L2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/><line x1="6" y1="16" x2="6.01" y2="16"/><line x1="10" y1="16" x2="10.01" y2="16"/></svg></div>
          <div class="platform-name">Docker</div>
          <div class="platform-desc">One command, runs anywhere</div>
          <div class="platform-cmd"><code title="docker run -p 4545:4545 ghcr.io/librefang/librefang">docker run -p 4545:4545 ghcr.io/librefang/librefang</code><button class="copy-btn" onclick="event.preventDefault();event.stopPropagation();copyText(this,'docker run -p 4545:4545 ghcr.io/librefang/librefang')">Copy</button></div>
        </a>
      </div>

      <div style="margin-top:24px;margin-bottom:16px;font-weight:600;font-size:1rem;color:var(--dim);">Install locally</div>
      <div class="platform-grid">
        <a class="platform-card accent-blue" href="https://github.com/librefang/librefang/releases/latest" target="_blank" rel="noopener">
          <div class="platform-icon"><svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3" width="20" height="14" rx="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></svg></div>
          <div class="platform-name">macOS</div>
          <div class="platform-desc">Homebrew or download binary</div>
          <div class="platform-cmd"><code title="brew install librefang/tap/librefang">brew install librefang/tap/librefang</code><button class="copy-btn" onclick="event.preventDefault();event.stopPropagation();copyText(this,'brew install librefang/tap/librefang')">Copy</button></div>
        </a>

        <a class="platform-card accent-blue" href="https://github.com/librefang/librefang/releases/latest" target="_blank" rel="noopener">
          <div class="platform-icon"><svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M12 2L2 7l10 5 10-5-10-5z"/><path d="M2 17l10 5 10-5"/><path d="M2 12l10 5 10-5"/></svg></div>
          <div class="platform-name">Linux</div>
          <div class="platform-desc">Install script or download binary</div>
          <div class="platform-cmd"><code title="curl -fsSL https://librefang.ai/install.sh | sh">curl -fsSL https://librefang.ai/install.sh | sh</code><button class="copy-btn" onclick="event.preventDefault();event.stopPropagation();copyText(this,'curl -fsSL https://librefang.ai/install.sh | sh')">Copy</button></div>
        </a>

        <a class="platform-card accent-blue" href="https://github.com/librefang/librefang/releases/latest" target="_blank" rel="noopener">
          <div class="platform-icon"><svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8"/><path d="M12 17v4"/><path d="M7.5 10l2 2 4-4"/></svg></div>
          <div class="platform-name">Windows</div>
          <div class="platform-desc">PowerShell installer or .msi</div>
          <div class="platform-cmd"><code title="irm https://librefang.ai/install.ps1 | iex">irm https://librefang.ai/install.ps1 | iex</code><button class="copy-btn" onclick="event.preventDefault();event.stopPropagation();copyText(this,'irm https://librefang.ai/install.ps1 | iex')">Copy</button></div>
        </a>
      </div>
    </div>

    <!-- Fly.io deploy form (hidden until selected) -->
    <div id="fly-deploy" style="display:none;">
      <button class="back-btn" onclick="showPlatforms()">&larr; Back to platforms</button>

      <div id="form-section">
        <div class="free-note">
          A free LLM (Step 3.5 Flash via OpenRouter) is pre-configured. Your instance works out of the box &mdash; no API keys required.
        </div>

        <div class="card">
          <div class="step">
            <div class="step-num">1</div>
            <div class="step-content">
              <div class="step-title">Get a Fly.io API Token</div>
              <div class="step-desc">
                <a href="https://fly.io/app/sign-up" target="_blank" rel="noopener">Sign up</a> or
                <a href="https://fly.io/app/sign-in" target="_blank" rel="noopener">log in</a> to Fly.io, then go to
                <a href="https://fly.io/user/personal_access_tokens" target="_blank" rel="noopener">Personal Access Tokens</a> and create a new token.
              </div>
            </div>
          </div>
          <div class="step">
            <div class="step-num">2</div>
            <div class="step-content">
              <div class="step-title">Paste and deploy</div>
              <div class="step-desc">Your token is only sent to the Fly.io API and is never stored on our servers.</div>
            </div>
          </div>
        </div>

        <div class="card">
          <div class="field">
            <label>Fly.io API Token <span style="color:var(--red)">*</span></label>
            <input type="password" id="token" placeholder="fo1_xxxxxxxxxxxx" autocomplete="off" />
          </div>

          <button class="btn" id="deployBtn" onclick="deploy()">Deploy to Fly.io</button>

          <div class="progress" id="progress">
            <div class="progress-step" id="ps-auth"><span class="icon"></span> Verifying token...</div>
            <div class="progress-step" id="ps-app"><span class="icon"></span> Creating app...</div>
            <div class="progress-step" id="ps-net"><span class="icon"></span> Allocating IP addresses...</div>
            <div class="progress-step" id="ps-vol"><span class="icon"></span> Creating persistent volume...</div>
            <div class="progress-step" id="ps-machine"><span class="icon"></span> Launching machine with Step 3.5 Flash...</div>
          </div>

          <div class="error-msg" id="error"></div>
        </div>
      </div>

      <div class="result" id="result">
        <div class="result-success">
          <h2>Deployed!</h2>
          <p style="color:var(--dim); margin-bottom: 16px;">
            Your LibreFang instance is starting up (1-2 min).<br>
            Free LLM (Step 3.5 Flash) is pre-configured and ready to use.
          </p>
          <a class="result-link" id="appLink" href="#" target="_blank">Open Dashboard</a>
          <a class="result-link secondary" id="flyLink" href="#" target="_blank">Fly.io Console</a>
          <div class="result-info" id="resultInfo"></div>
        </div>
      </div>

      <div class="card">
        <div style="font-weight:600;margin-bottom:12px;">Troubleshooting</div>
        <details style="margin-bottom:8px;">
          <summary style="color:var(--dim);font-size:0.85rem;cursor:pointer;">Cannot create Personal Access Token (SSO error)</summary>
          <div style="color:var(--dim);font-size:0.85rem;line-height:1.6;padding:8px 0 0 16px;">
            If you see: <em>"Access Tokens cannot be created because an organization requires SSO"</em><br>
            Use a per-org token instead. Run in your terminal:<br>
            <code style="color:var(--green);background:var(--bg);padding:2px 6px;border-radius:4px;">flyctl tokens org &lt;your-org-name&gt;</code><br>
            Then paste the generated token above.
          </div>
        </details>
        <details style="margin-bottom:8px;">
          <summary style="color:var(--dim);font-size:0.85rem;cursor:pointer;">Deploy failed: image not found</summary>
          <div style="color:var(--dim);font-size:0.85rem;line-height:1.6;padding:8px 0 0 16px;">
            The Docker image <code style="color:var(--green);">ghcr.io/librefang/librefang:latest</code> is built on each release.<br>
            If no release has been published yet, use the terminal deploy script below &mdash; it builds from source.
          </div>
        </details>
        <details>
          <summary style="color:var(--dim);font-size:0.85rem;cursor:pointer;">How to add or change LLM provider after deploy?</summary>
          <div style="color:var(--dim);font-size:0.85rem;line-height:1.6;padding:8px 0 0 16px;">
            <code style="color:var(--green);background:var(--bg);padding:2px 6px;border-radius:4px;">flyctl secrets set OPENAI_API_KEY=sk-... --app your-app-name</code><br>
            Then edit <code style="color:var(--green);">/data/config.toml</code> via <code style="color:var(--green);">flyctl ssh console</code> to update the default model.
          </div>
        </details>
      </div>
    </div>

    <div class="card" style="text-align:center;">
      <div style="color:var(--dim);font-size:0.85rem;margin-bottom:12px;">Or deploy from your terminal:</div>
      <div style="background:#0d0d14;border:1px solid var(--border);border-radius:10px;padding:14px 18px;display:flex;align-items:center;justify-content:space-between;gap:12px;overflow-x:auto;">
        <code style="color:var(--green);white-space:nowrap;font-size:0.85rem;"><span style="color:var(--dim);user-select:none;">$ </span>curl -sL https://raw.githubusercontent.com/librefang/librefang/main/deploy/fly/deploy.sh | bash</code>
        <button onclick="copyCmd(this)" style="background:var(--accent);color:white;border:none;border-radius:8px;padding:6px 14px;font-size:0.8rem;cursor:pointer;white-space:nowrap;flex-shrink:0;">Copy</button>
      </div>
    </div>

    <div class="footer">
      <a href="https://github.com/librefang/librefang">GitHub</a> &bull;
      <a href="https://librefang.ai">Website</a> &bull;
      <a href="https://discord.gg/DzTYqAZZmc">Discord</a>
      <p style="margin-top:8px;">LibreFang &mdash; Libre Agent Operating System</p>
    </div>
  </div>

  <script>
    function showFlyDeploy() {
      document.getElementById('platform-selection').style.display = 'none';
      document.getElementById('fly-deploy').style.display = 'block';
    }

    function showPlatforms() {
      document.getElementById('fly-deploy').style.display = 'none';
      document.getElementById('platform-selection').style.display = 'block';
    }

    async function deploy() {
      const token = document.getElementById('token').value.trim();
      if (!token) { showError('Please enter your Fly.io API Token.'); return; }

      const btn = document.getElementById('deployBtn');
      const progress = document.getElementById('progress');
      const errorEl = document.getElementById('error');

      btn.disabled = true;
      btn.textContent = 'Deploying...';
      btn.classList.add('deploying');
      errorEl.classList.remove('show');
      progress.classList.add('show');

      const steps = ['ps-auth', 'ps-app', 'ps-net', 'ps-vol', 'ps-machine'];
      let currentStep = 0;
      activateStep(steps[0]);

      const stepInterval = setInterval(() => {
        if (currentStep < steps.length - 1) {
          doneStep(steps[currentStep]);
          currentStep++;
          activateStep(steps[currentStep]);
        }
      }, 1500);

      try {
        const res = await fetch('/api/deploy', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ token }),
        });

        clearInterval(stepInterval);
        const data = await res.json();

        if (!res.ok || data.error) {
          throw new Error(data.error || 'Deployment failed');
        }

        steps.forEach(s => doneStep(s));

        document.getElementById('form-section').style.display = 'none';
        const result = document.getElementById('result');
        result.classList.add('show');
        document.getElementById('appLink').href = data.url;
        document.getElementById('flyLink').href = data.dashboardUrl;
        document.getElementById('resultInfo').innerHTML =
          'App: <code>' + data.appName + '</code> &bull; Region: <code>' + data.region + '</code><br>' +
          'Model: <code>Step 3.5 Flash (free)</code><br>' +
          'Upgrade model: <code>flyctl secrets set OPENAI_API_KEY=sk-... --app ' + data.appName + '</code>';
      } catch (err) {
        clearInterval(stepInterval);
        showError(err.message);
        btn.disabled = false;
        btn.textContent = 'Deploy to Fly.io';
        btn.classList.remove('deploying');
        progress.classList.remove('show');
        steps.forEach(s => resetStep(s));
      }
    }

    function activateStep(id) {
      const el = document.getElementById(id);
      el.classList.add('active');
      el.querySelector('.icon').innerHTML = '<span class="spinner"></span>';
    }
    function doneStep(id) {
      const el = document.getElementById(id);
      el.classList.remove('active');
      el.classList.add('done');
      el.querySelector('.icon').textContent = '\\u2713';
    }
    function resetStep(id) {
      const el = document.getElementById(id);
      el.classList.remove('active', 'done');
      el.querySelector('.icon').textContent = '';
    }
    function copyText(btn, text) {
      navigator.clipboard.writeText(text).then(() => {
        btn.textContent = "Copied!";
        btn.style.color = "var(--green)";
        btn.style.borderColor = "var(--green)";
        setTimeout(() => { btn.textContent = "Copy"; btn.style.color = ""; btn.style.borderColor = ""; }, 2000);
      });
    }
    function copyCmd(btn) {
      navigator.clipboard.writeText("curl -sL https://raw.githubusercontent.com/librefang/librefang/main/deploy/fly/deploy.sh | bash").then(() => {
        btn.textContent = "Copied!";
        btn.style.background = "var(--green)";
        btn.style.color = "#0a0a0f";
        setTimeout(() => { btn.textContent = "Copy"; btn.style.background = "var(--accent)"; btn.style.color = "white"; }, 2000);
      });
    }
    function showError(msg) {
      const el = document.getElementById('error');
      el.textContent = msg;
      el.classList.add('show');
    }
  </script>
</body>
</html>`;
