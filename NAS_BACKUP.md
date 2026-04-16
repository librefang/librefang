# NAS `/data` → Git backup (fork-only)

Daily backup of the LibreFang NAS deployment's `/data` volume to a private
Gitea repo. Runs inside the container under PM2 so it survives Lazycat NAS
reboots (which reset the host OS).

- **Repo:** `https://git.federicoliva.it/fede91it/librefang-nas-state` (private)
- **Schedule:** every day at `03:30` local (NAS timezone)
- **Scheduler:** PM2 cron — `librefang-backup` process inside
  `cloudlazycatapplibrefang-librefang-1`

## Why PM2 (and not systemd on the host)

Lazycat is an immutable-host appliance. Anything written under
`/etc/systemd/system/` on the host is wiped on reboot. Only the btrfs
subvolume mounted at `/root`, `/data`, and `/home/librefang` inside the
container survives — that subvolume is `/appvar/cloud.lazycat.app.librefang`
on `/dev/nvme1n1p1`.

PM2 already runs in the container (it supervises the WhatsApp gateway),
its config lives in `/root/.pm2/dump.pm2` (persisted), and the
`entrypoint.sh` that ships with `fliva/librefang:latest` calls
`pm2 resurrect` on every container start. Registering the backup as a PM2
app with `--cron "30 3 * * *"` is therefore the one setup that persists
across every restart mode:

| Event | Survives? | Why |
|---|---|---|
| `lzc-docker restart` | ✅ | same container, same rootfs |
| `lzc-docker compose up -d` (rebuild image) | ✅ | `/data` + `/root` are btrfs volumes, not overlay rootfs |
| NAS reboot | ✅ | Lazycat re-mounts `/appvar/…` subvolume into the container |
| App uninstall via Lazycat store | ❌ | subvolume removed — this is the only scenario that wipes the backup infra |

## What gets backed up

All under `/data/` inside the container, staged into
`/data/backup-repo/data/` and `/data/backup-repo/sql/`.

### Included

- Config: `config.toml`, its `.bak*` copies, `aliases.toml`,
  `integrations.toml`, `schema.toml`, `daemon.json`, `cron_jobs.json`,
  `message_journal.jsonl`, `secrets.env`, `hand_state.json`,
  `.clawhub-config.json`.
- Content dirs: `agents/`, `hands/`, `skills/`, `workflows/`,
  `workspaces/`, `channels/`, `integrations/`, `plugins/`,
  `providers/`, `registry/`, `packages/`, `dashboard/`, `mempalace/`,
  `.clawhub/`.
- WhatsApp gateway minimum set: `auth_store/` (Baileys/Signal session —
  losing it means re-scanning the QR code from scratch),
  `ecosystem.config.cjs`, `package.json`, `package-lock.json`,
  `index.js`, `lib/`, `scripts/`.
- SQLite databases, dumped to SQL text (diffable, restore via
  `sqlite3 new.db < dump.sql`):
  - `sql/librefang.sql` ← `/data/data/librefang.db`
  - `sql/openfang.sql` ← `/data/data/openfang.db`
  - `sql/wa-messages.sql` ← `/data/whatsapp-gateway/messages.db`
  - `sql/energy_prices.sql` ← `/data/skills/energy-monitor/data/energy_prices.db`

### Excluded on purpose

| Path / pattern | Why |
|---|---|
| `/data/librefang/` | 19 GB Claude Code session store — reconstructable on next run |
| `/data/npm-global/` | ~155 MB package cache — reconstructable |
| `node_modules/` (any depth) | reconstructable via `npm install` |
| `media_cache/` (WA gateway) | large + ephemeral voice notes |
| `logs/`, `*.log`, `*.pid`, `*.hash` | runtime cruft |
| `.cache/`, `__pycache__/`, `.pytest_cache/` | tooling cruft |

## Files on disk

Inside the container (all under the persisted btrfs subvolume):

```
/data/backup-repo/
├── .git/                    ← Gitea remote bound here
├── .gitignore               ← defensive excludes
├── README.md                ← repo-side restore instructions
├── backup.sh                ← the actual backup script (run by PM2)
├── dump_sqlite.py           ← .db → .sql helper (no sqlite3 CLI in container)
├── data/                    ← staged copy of /data subset
└── sql/                     ← SQLite text dumps
/root/.git-credentials       ← `https://fede91it:TOKEN@git.federicoliva.it`, chmod 600
/root/.gitconfig             ← credential.helper=store, user.name/email
/root/.pm2/dump.pm2          ← PM2 process list (includes librefang-backup)
```

On the host (managed by Lazycat, not us): nothing. The previous
`/etc/systemd/system/librefang-backup.{service,timer}` files were removed
when we migrated to PM2.

## Flow

```
  NAS reboot / container start
          │
          ▼
  entrypoint.sh runs
          │
          ▼
  pm2 resurrect  ← reads /root/.pm2/dump.pm2
          │
          ▼
  librefang-backup registered (status: stopped, cron_restart: "30 3 * * *")
          │
          ▼ 03:30 local
  PM2 starts /data/backup-repo/backup.sh
          │
          ├─ copy static config files from /data/ into /data/backup-repo/data/
          ├─ tar-pipe content dirs with --exclude=node_modules etc.
          ├─ python3 dump_sqlite.py → /data/backup-repo/sql/*.sql
          ├─ strip any nested .git dirs (cloned skills)
          ├─ git add -A
          ├─ git commit -m "auto backup <ISO-UTC>"  (skipped if no diff)
          └─ git push origin main
          │
          ▼ exit 0
  PM2 leaves process in "stopped" state until next cron tick
```

## Operator commands

All via SSH to the NAS (`root@192.168.8.115`, credentials in
`~/.claude-servizi/projects/-home-fede9-Progetti-librefang/.env.nas`).

```bash
source ~/.claude-servizi/projects/-home-fede9-Progetti-librefang/.env.nas

# Force a backup right now
sshpass -p "$NAS_PASS" ssh root@192.168.8.115 \
  'lzc-docker exec cloudlazycatapplibrefang-librefang-1 pm2 restart librefang-backup'

# Tail the last run's log
sshpass -p "$NAS_PASS" ssh root@192.168.8.115 \
  'lzc-docker exec cloudlazycatapplibrefang-librefang-1 pm2 logs librefang-backup --nostream --lines 100'

# Show PM2 status line for the backup job
sshpass -p "$NAS_PASS" ssh root@192.168.8.115 \
  'lzc-docker exec cloudlazycatapplibrefang-librefang-1 pm2 list | grep librefang-backup'

# Inspect what's staged in the repo working tree (without pushing)
sshpass -p "$NAS_PASS" ssh root@192.168.8.115 \
  'lzc-docker exec cloudlazycatapplibrefang-librefang-1 bash -c "cd /data/backup-repo && git status && git log --oneline -5"'
```

## Restore

From a clean container (after disaster), inside the container:

```bash
git clone https://fede91it:<TOKEN>@git.federicoliva.it/fede91it/librefang-nas-state.git /tmp/restore

# Static config + content
cp -a /tmp/restore/data/config.toml              /data/config.toml
cp -a /tmp/restore/data/secrets.env              /data/secrets.env
cp -a /tmp/restore/data/agents                   /data/
cp -a /tmp/restore/data/workspaces               /data/
cp -a /tmp/restore/data/whatsapp-gateway/auth_store /data/whatsapp-gateway/
# ...etc for every dir you need

# SQLite DBs
sqlite3 /data/data/librefang.db            < /tmp/restore/sql/librefang.sql
sqlite3 /data/whatsapp-gateway/messages.db < /tmp/restore/sql/wa-messages.sql
```

Then restart the daemon.

## Changing the schedule

Edit the cron expression in PM2, then save:

```bash
sshpass -p "$NAS_PASS" ssh root@192.168.8.115 \
  'lzc-docker exec cloudlazycatapplibrefang-librefang-1 bash -c \
    "pm2 delete librefang-backup && \
     pm2 start /data/backup-repo/backup.sh \
       --name librefang-backup --cron \"30 3 * * *\" \
       --no-autorestart --interpreter bash && \
     pm2 save"'
```

## Rotating the Gitea token

1. Generate a new token in Gitea (Settings → Applications, scope
   `write:repository`).
2. Replace the old one in the credentials file:
   ```bash
   sshpass -p "$NAS_PASS" ssh root@192.168.8.115 \
     'lzc-docker exec cloudlazycatapplibrefang-librefang-1 bash -c \
       "echo \"https://fede91it:NEWTOKEN@git.federicoliva.it\" > /root/.git-credentials && chmod 600 /root/.git-credentials"'
   ```
3. Revoke the old token in Gitea.
4. Next backup run will authenticate with the new token.

## Sensitivity

`data/secrets.env` contains all API keys (Anthropic, OpenAI, Groq,
Telegram bot token, etc.), and `data/whatsapp-gateway/auth_store/`
contains the live Signal/Baileys session credentials. **The Gitea repo
must stay private.** If the token or the repo ever leaks, rotate every
key in `secrets.env` and re-pair the WhatsApp device.
