# Skill marketplaces: when a hub stops answering with JSON

LibreFang reads skills from four remote hubs — ClawHub, the ClawHub China mirror, Skillhub, and the FangHub registry (which is a local checkout, not an HTTP API, and is not covered here).
This page is about one specific way the first three fail, because it does not look like a failure from the outside.

## The failure

A hub's API host is retired while the CDN in front of it keeps serving.
Every path then answers `200 OK` with the marketing single-page-app shell instead of the JSON the path used to return.
Nothing times out and nothing returns an error status, so the daemon happily hands the body to `serde_json`, which reports `expected value at line 1 column 1` — a parser's complaint about a `<`, offered to the reader as the entire explanation for why the Skills page is empty.

The daemon separates this from a genuinely malformed body.
A JSON document never begins with `<`, so a leading `<` — after an optional UTF-8 BOM and any whitespace — identifies "this is a webpage" without guessing.
That check lives in `librefang_skills::looks_like_markup`, and every remote marketplace read goes through `librefang_skills::parse_marketplace_json`, which turns it into `SkillError::MarketplaceUnavailable`.
A truncated or corrupted body stays `SkillError::Network`, because that one is worth a retry and this one is not.

## What you see

Every marketplace endpoint answers `503 Service Unavailable` with the condition spelled out, naming the operation and the URL that answered:

```
GET /api/clawhub/search        503   Marketplace unavailable: ClawHub search at … answered with a webpage instead of JSON …
GET /api/clawhub/browse        503
GET /api/clawhub/skill/{slug}  503
GET /api/clawhub/skill/{slug}/code
                               503
POST /api/clawhub/install      503
```

and identically for `/api/clawhub-cn/…` and `/api/skillhub/…`.

One status for all of them is the point.
Detail used to answer `404`, which told the reader the skill does not exist — a claim the daemon is in no position to make when the hub never answered as a marketplace.
Install used to answer `500`, whose body is then scrubbed to `Internal server error` before it leaves the process, discarding the one message an operator could act on.

The dashboard renders `503` (and `502`, for a daemon that predates this) as a single offline state for the hub rather than a load error with a parser transcript under it.

Skills already installed on disk are unaffected: nothing about a dead hub touches local execution, and `librefang skill install` from a local path keeps working.

## Pointing at a mirror

Each hub's endpoints are overridable, so a mirror can be adopted without recompiling.
Set these before starting the daemon; they are read when a client is constructed, so a restart is enough.

| Variable | Default | Feeds |
| --- | --- | --- |
| `LIBREFANG_CLAWHUB_URL` | `https://clawhub.ai/api/v1` | ClawHub search, browse, detail, source, install |
| `LIBREFANG_CLAWHUB_CN_URL` | `https://mirror-cn.clawhub.com/api/v1` | The same five endpoints on the China mirror |
| `LIBREFANG_SKILLHUB_URL` | `https://skillhub.tencent.com/api/v1` | Skillhub search and detail |
| `LIBREFANG_SKILLHUB_INDEX_URL` | `https://skillhub-1388575217.cos.ap-guangzhou.myqcloud.com/skills.json` | Skillhub browse, and the version lookup install starts from |
| `LIBREFANG_SKILLHUB_COS_URL` | `https://skillhub-1388575217.cos.accelerate.myqcloud.com` | Skillhub archive downloads |

Skillhub needs three because it is three hosts: an API, a static index, and object storage.
Overriding only the API base leaves browse and install still aimed at the dead host, which is the opposite of what an override is for — so a Skillhub mirror means setting all three, or none.

A variable set to whitespace is treated as unset rather than as a request to fetch from the empty string.
Trailing slashes are trimmed.

No replacement host for any of the three defaults has been verified, so LibreFang ships the original URLs and leaves the choice of mirror to you.
