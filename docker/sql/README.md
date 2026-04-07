# Schema Management

All database schema (tables, RPCs, RLS policies, indexes, triggers) is managed
by Qwntik migrations. Do not add SQL files here.

## Applying the schema

```bash
# Against local Supabase (dev)
cd <qwntik-repo>
pnpm supabase:web:reset

# Against SupaScale (production)
supabase db push --db-url <supascale-connection-string>
```

## What lives here vs in Qwntik

| This repo (`docker/`) | Qwntik (`apps/web/supabase/`) |
|-----------------------|-------------------------------|
| `Dockerfile.supabase-ruvector` — builds custom Postgres image with ruvector .so | `migrations/` — all tables, RLS, RPCs, indexes |
| `docker-compose.yml` — local dev compose | `config.toml` — Supabase config |
| First-boot init SQL (`zz-ruvector-init.sql` baked into image) — only runs `CREATE EXTENSION ruvector` + tuning | Everything else |

## First-boot vs migrations

The `zz-ruvector-init.sql` baked into the Dockerfile runs once on first boot (empty data dir).
It only enables the ruvector extension and tunes settings — no tables, no RPCs.
All schema comes from Qwntik migrations which run after the instance is up.
