---
name: api-gateway
description: |
  Connect to external services through Maton-managed API routes.
  Use this skill only after the user names the target app, account, and task.
  Start with read/list calls when possible and follow the app-specific reference before any change.
compatibility: Requires network access and Maton account setup
metadata:
  author: maton
  version: "1.0"
  clawdbot:
    emoji: 🧠
    homepage: "https://maton.ai"
---

# API Gateway

Managed API routing for third-party services, provided by [Maton](https://maton.ai). Use this only for a user-requested app, account, and task.

## Quick Start

```bash
# List Slack channels (read-only)
python <<'EOF'
import urllib.request, os, json
req = urllib.request.Request('https://api.maton.ai/slack/api/conversations.list?types=public_channel&limit=10')
req.add_header('Authorization', f'Bearer {os.environ["MATON_API_KEY"]}')
print(json.dumps(json.load(urllib.request.urlopen(req)), indent=2))
EOF
```


## Routing

Use `https://api.maton.ai/` with the app-prefixed routes documented in the examples below or in the matching reference file.

Read-only route examples:

```text
https://api.maton.ai/slack/api/conversations.list?types=public_channel&limit=10
https://api.maton.ai/google-mail/gmail/v1/users/me/messages
```

The first path segment is the app identifier listed in Supported Services. For Gmail, use `/google-mail/gmail/v1/users/me/messages`.

## Authentication

Use `MATON_API_KEY` from the environment when constructing the request's Authorization header, as shown in the examples. Never print or copy the key.

**IMPORTANT:** Treat `MATON_API_KEY` as a secret. Do not log it, paste it into prompts, or expose it in shared files or command output. Authorize only the services and scopes needed for the current task, revoke unused connections, and rotate the key if exposed.

**Environment Variable:** You can set your API key as the `MATON_API_KEY` environment variable:

```bash
export MATON_API_KEY="YOUR_API_KEY"
```

## Getting Your API Key

1. Sign in or create an account at [maton.ai](https://maton.ai)
2. Go to [maton.ai/settings](https://maton.ai/settings)
3. Click the copy button on the right side of API Key section to copy it

## Connection Management

Connection management uses a separate base URL: `https://api.maton.ai`

### List Connections

```bash
python <<'EOF'
import urllib.request, os, json
req = urllib.request.Request('https://api.maton.ai/connections?app=slack&status=ACTIVE')
req.add_header('Authorization', f'Bearer {os.environ["MATON_API_KEY"]}')
print(json.dumps(json.load(urllib.request.urlopen(req)), indent=2))
EOF
```

**Query Parameters (optional):**
- `app` - Filter by service name (e.g., `slack`, `hubspot`, `salesforce`)
- `status` - Filter by connection status (`ACTIVE`, `PENDING`, `FAILED`)

**Response:**
```json
{
  "connections": [
    {
      "connection_id": "{connection_id}",
      "status": "ACTIVE",
      "creation_time": "2025-12-08T07:20:53.488460Z",
      "last_updated_time": "2026-01-31T20:03:32.593153Z",
      "url": "https://connect.maton.ai/?session_token=5e9...",
      "app": "slack",
      "method": "OAUTH2",
      "metadata": {}
    }
  ]
}
```

### Create Connection

```bash
python <<'EOF'
import urllib.request, os, json
data = json.dumps({'app': 'slack'}).encode()
req = urllib.request.Request('https://api.maton.ai/connections', data=data, method='POST')
req.add_header('Authorization', f'Bearer {os.environ["MATON_API_KEY"]}')
req.add_header('Content-Type', 'application/json')
print(json.dumps(json.load(urllib.request.urlopen(req)), indent=2))
EOF
```

**Request Body:**
- `app` (required) - Service name (e.g., `slack`, `notion`)
- `method` (optional) - Connection method (`API_KEY`, `BASIC`, `OAUTH1`, `OAUTH2`, `MCP`)

### Get Connection

```bash
python <<'EOF'
import urllib.request, os, json
req = urllib.request.Request('https://api.maton.ai/connections/{connection_id}')
req.add_header('Authorization', f'Bearer {os.environ["MATON_API_KEY"]}')
print(json.dumps(json.load(urllib.request.urlopen(req)), indent=2))
EOF
```

**Response:**
```json
{
  "connection": {
    "connection_id": "{connection_id}",
    "status": "ACTIVE",
    "creation_time": "2025-12-08T07:20:53.488460Z",
    "last_updated_time": "2026-01-31T20:03:32.593153Z",
    "url": "https://connect.maton.ai/?session_token=5e9...",
    "app": "slack",
    "metadata": {}
  }
}
```

Open the returned URL in a browser to complete service authorization.

### Delete Connection

```bash
python <<'EOF'
import urllib.request, os, json
req = urllib.request.Request('https://api.maton.ai/connections/{connection_id}', method='DELETE')
req.add_header('Authorization', f'Bearer {os.environ["MATON_API_KEY"]}')
print(json.dumps(json.load(urllib.request.urlopen(req)), indent=2))
EOF
```

### Specifying Connection

If you have multiple connections for the same app, you can specify which connection to use by adding the `Maton-Connection` header with the connection ID:

```bash
python <<'EOF'
import urllib.request, os, json
req = urllib.request.Request('https://api.maton.ai/slack/api/conversations.list?types=public_channel&limit=10')
req.add_header('Authorization', f'Bearer {os.environ["MATON_API_KEY"]}')
req.add_header('Maton-Connection', '{connection_id}')
print(json.dumps(json.load(urllib.request.urlopen(req)), indent=2))
EOF
```

If you have multiple connections, always include this header to ensure requests go to the intended account.

## Security & Permissions

- Access is scoped to the specific third-party service connected through each Maton connection and the scopes the user authorized.
- **Use least privilege.** Connect only the services needed for the current task. Prefer read-only scopes and revoke unused connections promptly.
- **Default to read/list calls.** Retrieve or list resources first to verify identifiers, account context, and current state before proposing any change.
- **All operations that modify data require explicit user approval.** Before executing any POST, PUT, PATCH, or DELETE call, confirm the target service, resource, payload, and intended effect with the user. This includes sending messages, creating records, modifying content, deleting resources, and triggering workflows.
- **High-impact operations require extra caution.** Actions such as bulk deletions, publishing content, sending emails/messages to external recipients, modifying billing or financial data, or changing permissions must be clearly described with specific resource identifiers and confirmed before execution.
- **Always specify the connection.** Use the `Maton-Connection` header to ensure requests go to the intended account, especially when the user has multiple connections for the same service.

## Supported Services

| Service | App Name | Service API Host |
|---------|----------|------------------|
| ActiveCampaign | `active-campaign` | `{account}.api-us1.com` |
| Acuity Scheduling | `acuity-scheduling` | `acuityscheduling.com` |
| Airtable | `airtable` | `api.airtable.com` |
| Apify | `apify` | `api.apify.com` |
| Apollo | `apollo` | `api.apollo.io` |
| Asana | `asana` | `app.asana.com` |
| Attio | `attio` | `api.attio.com` |
| Basecamp | `basecamp` | `3.basecampapi.com` |
| Baserow | `baserow` | `api.baserow.io` |
| beehiiv | `beehiiv` | `api.beehiiv.com` |
| Box | `box` | `api.box.com` |
| Brevo | `brevo` | `api.brevo.com` |
| Brave Search | `brave-search` | `api.search.brave.com` |
| Buffer | `buffer` | `api.buffer.com` |
| Calendly | `calendly` | `api.calendly.com` |
| Cal.com | `cal-com` | `api.cal.com` |
| CallRail | `callrail` | `api.callrail.com` |
| Chargebee | `chargebee` | `{subdomain}.chargebee.com` |
| ClickFunnels | `clickfunnels` | `{subdomain}.myclickfunnels.com` |
| ClickSend | `clicksend` | `rest.clicksend.com` |
| ClickUp | `clickup` | `api.clickup.com` |
| Clio | `clio` | `app.clio.com` |
| Clockify | `clockify` | `api.clockify.me` |
| Coda | `coda` | `coda.io` |
| Confluence | `confluence` | `api.atlassian.com` |
| CompanyCam | `companycam` | `api.companycam.com` |
| Cognito Forms | `cognito-forms` | `www.cognitoforms.com` |
| Constant Contact | `constant-contact` | `api.cc.email` |
| Dropbox | `dropbox` | `api.dropboxapi.com` |
| Dropbox Business | `dropbox-business` | `api.dropboxapi.com` |
| ElevenLabs | `elevenlabs` | `api.elevenlabs.io` |
| Eventbrite | `eventbrite` | `www.eventbriteapi.com` |
| Exa | `exa` | `api.exa.ai` |
| Facebook Page | `facebook-page` | `graph.facebook.com` |
| fal.ai | `fal-ai` | `queue.fal.run` |
| Fathom | `fathom` | `api.fathom.ai` |
| Firecrawl | `firecrawl` | `api.firecrawl.dev` |
| Firebase | `firebase` | `firebase.googleapis.com` |
| Fireflies | `fireflies` | `api.fireflies.ai` |
| Front | `front` | `api2.frontapp.com` |
| GetResponse | `getresponse` | `api.getresponse.com` |
| Grafana | `grafana` | User's Grafana instance |
| GitHub | `github` | `api.github.com` |
| Gumroad | `gumroad` | `api.gumroad.com` |
| Granola MCP | `granola` | `mcp.granola.ai` |
| Google Ads | `google-ads` | `googleads.googleapis.com` |
| Google BigQuery | `google-bigquery` | `bigquery.googleapis.com` |
| Google Analytics Admin | `google-analytics-admin` | `analyticsadmin.googleapis.com` |
| Google Analytics Data | `google-analytics-data` | `analyticsdata.googleapis.com` |
| Google Calendar | `google-calendar` | `www.googleapis.com` |
| Google Classroom | `google-classroom` | `classroom.googleapis.com` |
| Google Contacts | `google-contacts` | `people.googleapis.com` |
| Google Docs | `google-docs` | `docs.googleapis.com` |
| Google Drive | `google-drive` | `www.googleapis.com` |
| Google Forms | `google-forms` | `forms.googleapis.com` |
| Gmail | `google-mail` | `gmail.googleapis.com` |
| Google Merchant | `google-merchant` | `merchantapi.googleapis.com` |
| Google Meet | `google-meet` | `meet.googleapis.com` |
| Google Play | `google-play` | `androidpublisher.googleapis.com` |
| Google Search Console | `google-search-console` | `www.googleapis.com` |
| Google Sheets | `google-sheets` | `sheets.googleapis.com` |
| Google Slides | `google-slides` | `slides.googleapis.com` |
| Google Tasks | `google-tasks` | `tasks.googleapis.com` |
| Google Workspace Admin | `google-workspace-admin` | `admin.googleapis.com` |
| GoHighLevel (PIT) | `highlevel-pit` | `services.leadconnectorhq.com` |
| HubSpot | `hubspot` | `api.hubapi.com` |
| Instantly | `instantly` | `api.instantly.ai` |
| Jira | `jira` | `api.atlassian.com` |
| Jobber | `jobber` | `api.getjobber.com` |
| JotForm | `jotform` | `api.jotform.com` |
| Kaggle | `kaggle` | `api.kaggle.com` |
| Keap | `keap` | `api.infusionsoft.com` |
| Kibana | `kibana` | User's Kibana instance |
| Kit | `kit` | `api.kit.com` |
| Klaviyo | `klaviyo` | `a.klaviyo.com` |
| Lemlist | `lemlist` | `api.lemlist.com` |
| Linear | `linear` | `api.linear.app` |
| LinkedIn | `linkedin` | `api.linkedin.com` |
| Mailchimp | `mailchimp` | `{dc}.api.mailchimp.com` |
| MailerLite | `mailerlite` | `connect.mailerlite.com` |
| Mailgun | `mailgun` | `api.mailgun.net` |
| Make | `make` | `{zone}.make.com` |
| ManyChat | `manychat` | `api.manychat.com` |
| Manus | `manus` | `api.manus.ai` |
| Memelord | `memelord` | `www.memelord.com` |
| Meta Ads | `meta-ads` | `graph.facebook.com` |
| Microsoft Excel | `microsoft-excel` | `graph.microsoft.com` |
| Microsoft Teams | `microsoft-teams` | `graph.microsoft.com` |
| Microsoft To Do | `microsoft-to-do` | `graph.microsoft.com` |
| Monday.com | `monday` | `api.monday.com` |
| Motion | `motion` | `api.usemotion.com` |
| Netlify | `netlify` | `api.netlify.com` |
| Notion | `notion` | `api.notion.com` |
| Notion MCP | `notion` | `mcp.notion.com` |
| OneNote | `one-note` | `graph.microsoft.com` |
| OneDrive | `one-drive` | `graph.microsoft.com` |
| Outlook | `outlook` | `graph.microsoft.com` |
| PDF.co | `pdf-co` | `api.pdf.co` |
| Pipedrive | `pipedrive` | `api.pipedrive.com` |
| Podio | `podio` | `api.podio.com` |
| PostHog | `posthog` | `{subdomain}.posthog.com` |
| QuickBooks | `quickbooks` | `quickbooks.api.intuit.com` |
| Quo | `quo` | `api.openphone.com` |
| Reducto | `reducto` | `platform.reducto.ai` |
| Resend | `resend` | `api.resend.com` |
| Salesforce | `salesforce` | `{instance}.salesforce.com` |
| Sentry | `sentry` | `{subdomain}.sentry.io` |
| SharePoint | `sharepoint` | `graph.microsoft.com` |
| SignNow | `signnow` | `api.signnow.com` |
| Slack | `slack` | `slack.com` |
| Snapchat | `snapchat` | `adsapi.snapchat.com` |
| Square | `squareup` | `connect.squareup.com` |
| Squarespace | `squarespace` | `api.squarespace.com` |
| Stripe | `stripe` | `api.stripe.com` |
| Sunsama MCP | `sunsama` | MCP server |
| Supabase | `supabase` | `{project_ref}.supabase.co` |
| Systeme.io | `systeme` | `api.systeme.io` |
| Tally | `tally` | `api.tally.so` |
| Tavily | `tavily` | `api.tavily.com` |
| Telegram | `telegram` | `api.telegram.org` |
| TickTick | `ticktick` | `api.ticktick.com` |
| Todoist | `todoist` | `api.todoist.com` |
| Toggl Track | `toggl-track` | `api.track.toggl.com` |
| Trello | `trello` | `api.trello.com` |
| Twilio | `twilio` | `api.twilio.com` |
| Twenty CRM | `twenty` | `api.twenty.com` |
| Typeform | `typeform` | `api.typeform.com` |
| Unbounce | `unbounce` | `api.unbounce.com` |
| Vercel | `vercel` | `api.vercel.com` |
| Vimeo | `vimeo` | `api.vimeo.com` |
| WATI | `wati` | `{tenant}.wati.io` |
| WhatsApp Business | `whatsapp-business` | `graph.facebook.com` |
| WooCommerce | `woocommerce` | `{store-url}/wp-json/wc/v3` |
| WordPress.com | `wordpress` | `public-api.wordpress.com` |
| Wrike | `wrike` | `www.wrike.com` |
| Xero | `xero` | `api.xero.com` |
| YouTube | `youtube` | `www.googleapis.com` |
| Zoom | `zoom` | `api.zoom.us` |
| Zoom Admin | `zoom-admin` | `api.zoom.us` |
| Zoho Bigin | `zoho-bigin` | `www.zohoapis.com` |
| Zoho Bookings | `zoho-bookings` | `www.zohoapis.com` |
| Zoho Books | `zoho-books` | `www.zohoapis.com` |
| Zoho Calendar | `zoho-calendar` | `calendar.zoho.com` |
| Zoho CRM | `zoho-crm` | `www.zohoapis.com` |
| Zoho Inventory | `zoho-inventory` | `www.zohoapis.com` |
| Zoho Mail | `zoho-mail` | `mail.zoho.com` |
| Zoho People | `zoho-people` | `people.zoho.com` |
| Zoho Projects | `zoho-projects` | `projectsapi.zoho.com` |
| Zoho Recruit | `zoho-recruit` | `recruit.zoho.com` |

See [references/](https://github.com/maton-ai/api-gateway-skill/tree/main/references/) for detailed routing guides per provider:
- [ActiveCampaign](https://github.com/maton-ai/api-gateway-skill/tree/main/references/active-campaign/README.md) - Contacts, deals, tags, lists, automations, campaigns
- [Acuity Scheduling](https://github.com/maton-ai/api-gateway-skill/tree/main/references/acuity-scheduling/README.md) - Appointments, calendars, clients, availability
- [Airtable](https://github.com/maton-ai/api-gateway-skill/tree/main/references/airtable/README.md) - Records, bases, tables
- [Apify](https://github.com/maton-ai/api-gateway-skill/tree/main/references/apify/README.md) - Actors, runs, datasets, key-value stores, request queues, schedules
- [Apollo](https://github.com/maton-ai/api-gateway-skill/tree/main/references/apollo/README.md) - People search, enrichment, contacts
- [Asana](https://github.com/maton-ai/api-gateway-skill/tree/main/references/asana/README.md) - Tasks, projects, workspaces, webhooks
- [Attio](https://github.com/maton-ai/api-gateway-skill/tree/main/references/attio/README.md) - People, companies, records, tasks
- [Basecamp](https://github.com/maton-ai/api-gateway-skill/tree/main/references/basecamp/README.md) - Projects, to-dos, messages, schedules, documents
- [Baserow](https://github.com/maton-ai/api-gateway-skill/tree/main/references/baserow/README.md) - Database rows, fields, tables, batch operations
- [beehiiv](https://github.com/maton-ai/api-gateway-skill/tree/main/references/beehiiv/README.md) - Publications, subscriptions, posts, custom fields
- [Box](https://github.com/maton-ai/api-gateway-skill/tree/main/references/box/README.md) - Files, folders, collaborations, shared links
- [Brevo](https://github.com/maton-ai/api-gateway-skill/tree/main/references/brevo/README.md) - Contacts, email campaigns, transactional emails, templates
- [Brave Search](https://github.com/maton-ai/api-gateway-skill/tree/main/references/brave-search/README.md) - Web search, image search, news search, video search
- [Buffer](https://github.com/maton-ai/api-gateway-skill/tree/main/references/buffer/README.md) - Social media posts, channels, organizations, scheduling
- [Calendly](https://github.com/maton-ai/api-gateway-skill/tree/main/references/calendly/README.md) - Event types, scheduled events, availability, webhooks
- [Cal.com](https://github.com/maton-ai/api-gateway-skill/tree/main/references/cal-com/README.md) - Event types, bookings, schedules, availability slots, webhooks
- [CallRail](https://github.com/maton-ai/api-gateway-skill/tree/main/references/callrail/README.md) - Calls, trackers, companies, tags, analytics
- [Chargebee](https://github.com/maton-ai/api-gateway-skill/tree/main/references/chargebee/README.md) - Subscriptions, customers, invoices
- [ClickFunnels](https://github.com/maton-ai/api-gateway-skill/tree/main/references/clickfunnels/README.md) - Contacts, products, orders, courses, webhooks
- [ClickSend](https://github.com/maton-ai/api-gateway-skill/tree/main/references/clicksend/README.md) - SMS, MMS, voice messages, contacts, lists
- [ClickUp](https://github.com/maton-ai/api-gateway-skill/tree/main/references/clickup/README.md) - Tasks, lists, folders, spaces, webhooks
- [Clio](https://github.com/maton-ai/api-gateway-skill/tree/main/references/clio/README.md) - Matters, contacts, activities, tasks, calendar entries, documents
- [Clockify](https://github.com/maton-ai/api-gateway-skill/tree/main/references/clockify/README.md) - Time tracking, projects, clients, tasks, workspaces
- [Coda](https://github.com/maton-ai/api-gateway-skill/tree/main/references/coda/README.md) - Docs, pages, tables, rows, formulas, controls
- [Confluence](https://github.com/maton-ai/api-gateway-skill/tree/main/references/confluence/README.md) - Pages, spaces, blogposts, comments, attachments
- [CompanyCam](https://github.com/maton-ai/api-gateway-skill/tree/main/references/companycam/README.md) - Projects, photos, users, tags, groups, documents
- [Cognito Forms](https://github.com/maton-ai/api-gateway-skill/tree/main/references/cognito-forms/README.md) - Forms, entries, documents, files
- [Constant Contact](https://github.com/maton-ai/api-gateway-skill/tree/main/references/constant-contact/README.md) - Contacts, email campaigns, lists, tags, custom fields, segments, bulk activities, reporting
- [Dropbox](https://github.com/maton-ai/api-gateway-skill/tree/main/references/dropbox/README.md) - Files, folders, search, metadata, revisions, tags
- [Dropbox Business](https://github.com/maton-ai/api-gateway-skill/tree/main/references/dropbox-business/README.md) - Team members, groups, team folders, devices, audit logs
- [ElevenLabs](https://github.com/maton-ai/api-gateway-skill/tree/main/references/elevenlabs/README.md) - Text-to-speech, voice cloning, sound effects, audio processing
- [Eventbrite](https://github.com/maton-ai/api-gateway-skill/tree/main/references/eventbrite/README.md) - Events, venues, tickets, orders, attendees
- [Exa](https://github.com/maton-ai/api-gateway-skill/tree/main/references/exa/README.md) - Neural web search, content extraction, similar pages, AI answers, research tasks
- [fal.ai](https://github.com/maton-ai/api-gateway-skill/tree/main/references/fal-ai/README.md) - AI model inference (image generation, video, audio, upscaling)
- [Facebook Page](https://github.com/maton-ai/api-gateway-skill/tree/main/references/facebook-page/README.md) - Pages, posts, comments, insights, photos, videos, product catalogs
- [Fathom](https://github.com/maton-ai/api-gateway-skill/tree/main/references/fathom/README.md) - Meeting recordings, transcripts, summaries, webhooks
- [Firecrawl](https://github.com/maton-ai/api-gateway-skill/tree/main/references/firecrawl/README.md) - Web scraping, crawling, site mapping, web search
- [Firebase](https://github.com/maton-ai/api-gateway-skill/tree/main/references/firebase/README.md) - Projects, web apps, Android apps, iOS apps, configurations
- [Fireflies](https://github.com/maton-ai/api-gateway-skill/tree/main/references/fireflies/README.md) - Meeting transcripts, summaries, AskFred AI, channels
- [Front](https://github.com/maton-ai/api-gateway-skill/tree/main/references/front/README.md) - Conversations, messages, contacts, tags, inboxes, teammates
- [GetResponse](https://github.com/maton-ai/api-gateway-skill/tree/main/references/getresponse/README.md) - Campaigns, contacts, newsletters, autoresponders, tags, segments
- [Grafana](https://github.com/maton-ai/api-gateway-skill/tree/main/references/grafana/README.md) - Dashboards, data sources, folders, annotations, alerts, teams
- [GitHub](https://github.com/maton-ai/api-gateway-skill/tree/main/references/github/README.md) - Repositories, issues, pull requests, commits
- [Gumroad](https://github.com/maton-ai/api-gateway-skill/tree/main/references/gumroad/README.md) - Products, sales, subscribers, licenses, webhooks
- [Granola MCP](https://github.com/maton-ai/api-gateway-skill/tree/main/references/granola-mcp/README.md) - MCP-based interface for meeting notes, transcripts, queries
- [Google Ads](https://github.com/maton-ai/api-gateway-skill/tree/main/references/google-ads/README.md) - Campaigns, ad groups, GAQL queries
- [Google Analytics Admin](https://github.com/maton-ai/api-gateway-skill/tree/main/references/google-analytics-admin/README.md) - Reports, dimensions, metrics
- [Google Analytics Data](https://github.com/maton-ai/api-gateway-skill/tree/main/references/google-analytics-data/README.md) - Reports, dimensions, metrics
- [Google BigQuery](https://github.com/maton-ai/api-gateway-skill/tree/main/references/google-bigquery/README.md) - Datasets, tables, jobs, SQL queries
- [Google Calendar](https://github.com/maton-ai/api-gateway-skill/tree/main/references/google-calendar/README.md) - Events, calendars, free/busy
- [Google Classroom](https://github.com/maton-ai/api-gateway-skill/tree/main/references/google-classroom/README.md) - Courses, coursework, students, teachers, announcements
- [Google Contacts](https://github.com/maton-ai/api-gateway-skill/tree/main/references/google-contacts/README.md) - Contacts, contact groups, people search
- [Google Docs](https://github.com/maton-ai/api-gateway-skill/tree/main/references/google-docs/README.md) - Document creation, batch updates
- [Google Drive](https://github.com/maton-ai/api-gateway-skill/tree/main/references/google-drive/README.md) - Files, folders, permissions
- [Google Forms](https://github.com/maton-ai/api-gateway-skill/tree/main/references/google-forms/README.md) - Forms, questions, responses
- [Gmail](https://github.com/maton-ai/api-gateway-skill/tree/main/references/google-mail/README.md) - Messages, threads, labels
- [Google Meet](https://github.com/maton-ai/api-gateway-skill/tree/main/references/google-meet/README.md) - Spaces, conference records, participants
- [Google Merchant](https://github.com/maton-ai/api-gateway-skill/tree/main/references/google-merchant/README.md) - Products, inventories, promotions, reports
- [Google Play](https://github.com/maton-ai/api-gateway-skill/tree/main/references/google-play/README.md) - In-app products, subscriptions, reviews
- [Google Search Console](https://github.com/maton-ai/api-gateway-skill/tree/main/references/google-search-console/README.md) - Search analytics, sitemaps
- [Google Sheets](https://github.com/maton-ai/api-gateway-skill/tree/main/references/google-sheets/README.md) - Values, ranges, formatting
- [Google Slides](https://github.com/maton-ai/api-gateway-skill/tree/main/references/google-slides/README.md) - Presentations, slides, formatting
- [Google Tasks](https://github.com/maton-ai/api-gateway-skill/tree/main/references/google-tasks/README.md) - Task lists, tasks, subtasks
- [Google Workspace Admin](https://github.com/maton-ai/api-gateway-skill/tree/main/references/google-workspace-admin/README.md) - Users, groups, org units, domains, roles
- [GoHighLevel PIT](https://github.com/maton-ai/api-gateway-skill/tree/main/references/highlevel-pit/README.md) - Contacts, opportunities, calendars, conversations, locations, custom fields
- [HubSpot](https://github.com/maton-ai/api-gateway-skill/tree/main/references/hubspot/README.md) - Contacts, companies, deals
- [Instantly](https://github.com/maton-ai/api-gateway-skill/tree/main/references/instantly/README.md) - Campaigns, leads, accounts, email outreach
- [Jira](https://github.com/maton-ai/api-gateway-skill/tree/main/references/jira/README.md) - Issues, projects, JQL queries
- [Jobber](https://github.com/maton-ai/api-gateway-skill/tree/main/references/jobber/README.md) - Clients, jobs, invoices, quotes (GraphQL)
- [JotForm](https://github.com/maton-ai/api-gateway-skill/tree/main/references/jotform/README.md) - Forms, submissions, webhooks
- [Kaggle](https://github.com/maton-ai/api-gateway-skill/tree/main/references/kaggle/README.md) - Datasets, models, competitions, kernels
- [Keap](https://github.com/maton-ai/api-gateway-skill/tree/main/references/keap/README.md) - Contacts, companies, tags, tasks, opportunities, campaigns
- [Kibana](https://github.com/maton-ai/api-gateway-skill/tree/main/references/kibana/README.md) - Saved objects, dashboards, data views, spaces, alerts, fleet
- [Kit](https://github.com/maton-ai/api-gateway-skill/tree/main/references/kit/README.md) - Subscribers, tags, forms, sequences
- [Klaviyo](https://github.com/maton-ai/api-gateway-skill/tree/main/references/klaviyo/README.md) - Profiles, lists, campaigns, flows, events
- [Lemlist](https://github.com/maton-ai/api-gateway-skill/tree/main/references/lemlist/README.md) - Campaigns, leads, activities, schedules, unsubscribes
- [Linear](https://github.com/maton-ai/api-gateway-skill/tree/main/references/linear/README.md) - Issues, projects, teams, cycles (GraphQL)
- [LinkedIn](https://github.com/maton-ai/api-gateway-skill/tree/main/references/linkedin/README.md) - Profile, posts, shares, media uploads
- [Mailchimp](https://github.com/maton-ai/api-gateway-skill/tree/main/references/mailchimp/README.md) - Audiences, campaigns, templates, automations
- [MailerLite](https://github.com/maton-ai/api-gateway-skill/tree/main/references/mailerlite/README.md) - Subscribers, groups, campaigns, automations, forms
- [Mailgun](https://github.com/maton-ai/api-gateway-skill/tree/main/references/mailgun/README.md) - Domains, routes, templates, mailing lists, suppressions
- [Make](https://github.com/maton-ai/api-gateway-skill/tree/main/references/make/README.md) - Scenarios, organizations, teams, connections, data stores, hooks
- [ManyChat](https://github.com/maton-ai/api-gateway-skill/tree/main/references/manychat/README.md) - Subscribers, tags, flows, messaging
- [Manus](https://github.com/maton-ai/api-gateway-skill/tree/main/references/manus/README.md) - AI agent tasks, projects, files, webhooks
- [Memelord](https://github.com/maton-ai/api-gateway-skill/tree/main/references/memelord/README.md) - AI meme generation, video memes, template editing
- [Meta Ads](https://github.com/maton-ai/api-gateway-skill/tree/main/references/meta-ads/README.md) - Ad accounts, campaigns, ad sets, ads, creatives, insights
- [Microsoft Excel](https://github.com/maton-ai/api-gateway-skill/tree/main/references/microsoft-excel/README.md) - Workbooks, worksheets, ranges, tables, charts
- [Microsoft Teams](https://github.com/maton-ai/api-gateway-skill/tree/main/references/microsoft-teams/README.md) - Teams, channels, messages, members, chats
- [Microsoft To Do](https://github.com/maton-ai/api-gateway-skill/tree/main/references/microsoft-to-do/README.md) - Task lists, tasks, checklist items, linked resources
- [Monday.com](https://github.com/maton-ai/api-gateway-skill/tree/main/references/monday/README.md) - Boards, items, columns, groups (GraphQL)
- [Motion](https://github.com/maton-ai/api-gateway-skill/tree/main/references/motion/README.md) - Tasks, projects, workspaces, schedules
- [Netlify](https://github.com/maton-ai/api-gateway-skill/tree/main/references/netlify/README.md) - Sites, deploys, builds, DNS, environment variables
- [Notion](https://github.com/maton-ai/api-gateway-skill/tree/main/references/notion/README.md) - Pages, databases, blocks
- [Notion MCP](https://github.com/maton-ai/api-gateway-skill/tree/main/references/notion-mcp/README.md) - MCP-based interface for pages, databases, comments, teams, users
- [OneNote](https://github.com/maton-ai/api-gateway-skill/tree/main/references/one-note/README.md) - Notebooks, sections, section groups, pages via Microsoft Graph
- [OneDrive](https://github.com/maton-ai/api-gateway-skill/tree/main/references/one-drive/README.md) - Files, folders, drives, sharing
- [Outlook](https://github.com/maton-ai/api-gateway-skill/tree/main/references/outlook/README.md) - Mail, calendar, contacts
- [PDF.co](https://github.com/maton-ai/api-gateway-skill/tree/main/references/pdf-co/README.md) - PDF conversion, merge, split, edit, text extraction, barcodes
- [Pipedrive](https://github.com/maton-ai/api-gateway-skill/tree/main/references/pipedrive/README.md) - Deals, persons, organizations, activities
- [Podio](https://github.com/maton-ai/api-gateway-skill/tree/main/references/podio/README.md) - Organizations, workspaces, apps, items, tasks, comments
- [PostHog](https://github.com/maton-ai/api-gateway-skill/tree/main/references/posthog/README.md) - Product analytics, feature flags, session recordings, experiments, HogQL queries
- [QuickBooks](https://github.com/maton-ai/api-gateway-skill/tree/main/references/quickbooks/README.md) - Customers, invoices, reports
- [Quo](https://github.com/maton-ai/api-gateway-skill/tree/main/references/quo/README.md) - Calls, messages, contacts, conversations, webhooks
- [Reducto](https://github.com/maton-ai/api-gateway-skill/tree/main/references/reducto/README.md) - Document parsing, extraction, splitting, editing
- [Resend](https://github.com/maton-ai/api-gateway-skill/tree/main/references/resend/README.md) - Domains, audiences, contacts, webhooks
- [Salesforce](https://github.com/maton-ai/api-gateway-skill/tree/main/references/salesforce/README.md) - SOQL, sObjects, CRUD
- [SignNow](https://github.com/maton-ai/api-gateway-skill/tree/main/references/signnow/README.md) - Documents, templates, invites, e-signatures
- [SendGrid](https://github.com/maton-ai/api-gateway-skill/tree/main/references/sendgrid/README.md) - Contacts, templates, suppressions, statistics
- [Sentry](https://github.com/maton-ai/api-gateway-skill/tree/main/references/sentry/README.md) - Issues, events, projects, teams, releases
- [SharePoint](https://github.com/maton-ai/api-gateway-skill/tree/main/references/sharepoint/README.md) - Sites, lists, document libraries, files, folders, versions
- [Slack](https://github.com/maton-ai/api-gateway-skill/tree/main/references/slack/README.md) - Messages, channels, users
- [Snapchat](https://github.com/maton-ai/api-gateway-skill/tree/main/references/snapchat/README.md) - Ad accounts, campaigns, ad squads, ads, creatives, audiences
- [Square](https://github.com/maton-ai/api-gateway-skill/tree/main/references/squareup/README.md) - Customers, orders, catalog, inventory, invoices
- [Squarespace](https://github.com/maton-ai/api-gateway-skill/tree/main/references/squarespace/README.md) - Products, inventory, orders, profiles, transactions
- [Stripe](https://github.com/maton-ai/api-gateway-skill/tree/main/references/stripe/README.md) - Customers, subscriptions, account records
- [Sunsama MCP](https://github.com/maton-ai/api-gateway-skill/tree/main/references/sunsama-mcp/README.md) - MCP-based interface for tasks, calendar, backlog, objectives, time tracking
- [Supabase](https://github.com/maton-ai/api-gateway-skill/tree/main/references/supabase/README.md) - Database tables, auth users, storage buckets
- [Systeme.io](https://github.com/maton-ai/api-gateway-skill/tree/main/references/systeme/README.md) - Contacts, tags, courses, communities, webhooks
- [Tally](https://github.com/maton-ai/api-gateway-skill/tree/main/references/tally/README.md) - Forms, submissions, workspaces, webhooks
- [Tavily](https://github.com/maton-ai/api-gateway-skill/tree/main/references/tavily/README.md) - AI web search, content extraction, crawling, research tasks
- [Telegram](https://github.com/maton-ai/api-gateway-skill/tree/main/references/telegram/README.md) - Messages, chats, bots, updates, polls
- [TickTick](https://github.com/maton-ai/api-gateway-skill/tree/main/references/ticktick/README.md) - Tasks, projects, task lists
- [Todoist](https://github.com/maton-ai/api-gateway-skill/tree/main/references/todoist/README.md) - Tasks, projects, sections, labels, comments
- [Toggl Track](https://github.com/maton-ai/api-gateway-skill/tree/main/references/toggl-track/README.md) - Time entries, projects, clients, tags, workspaces
- [Trello](https://github.com/maton-ai/api-gateway-skill/tree/main/references/trello/README.md) - Boards, lists, cards, checklists
- [Twilio](https://github.com/maton-ai/api-gateway-skill/tree/main/references/twilio/README.md) - SMS, voice calls, phone numbers, messaging
- [Twenty CRM](https://github.com/maton-ai/api-gateway-skill/tree/main/references/twenty/README.md) - Companies, people, opportunities, notes, tasks
- [Typeform](https://github.com/maton-ai/api-gateway-skill/tree/main/references/typeform/README.md) - Forms, responses, insights
- [Unbounce](https://github.com/maton-ai/api-gateway-skill/tree/main/references/unbounce/README.md) - Landing pages, leads, accounts, sub-accounts, domains
- [Vercel](https://github.com/maton-ai/api-gateway-skill/tree/main/references/vercel/README.md) - Projects, deployments, domains, environment variables
- [Vimeo](https://github.com/maton-ai/api-gateway-skill/tree/main/references/vimeo/README.md) - Videos, folders, albums, comments, likes
- [WATI](https://github.com/maton-ai/api-gateway-skill/tree/main/references/wati/README.md) - WhatsApp messages, contacts, templates, interactive messages
- [WhatsApp Business](https://github.com/maton-ai/api-gateway-skill/tree/main/references/whatsapp-business/README.md) - Messages, templates, media
- [WooCommerce](https://github.com/maton-ai/api-gateway-skill/tree/main/references/woocommerce/README.md) - Products, orders, customers, coupons
- [WordPress.com](https://github.com/maton-ai/api-gateway-skill/tree/main/references/wordpress/README.md) - Posts, pages, sites, users, settings
- [Wrike](https://github.com/maton-ai/api-gateway-skill/tree/main/references/wrike/README.md) - Tasks, folders, projects, spaces, comments, timelogs, workflows
- [Xero](https://github.com/maton-ai/api-gateway-skill/tree/main/references/xero/README.md) - Contacts, invoices, reports
- [YouTube](https://github.com/maton-ai/api-gateway-skill/tree/main/references/youtube/README.md) - Videos, playlists, channels, subscriptions
- [Zoom](https://github.com/maton-ai/api-gateway-skill/tree/main/references/zoom/README.md) - Meetings, recordings, webinars, users
- [Zoom Admin](https://github.com/maton-ai/api-gateway-skill/tree/main/references/zoom-admin/README.md) - Users, meetings, webinars, recordings, account settings (admin scopes)
- [Zoho Bigin](https://github.com/maton-ai/api-gateway-skill/tree/main/references/zoho-bigin/README.md) - Contacts, companies, pipelines, products
- [Zoho Bookings](https://github.com/maton-ai/api-gateway-skill/tree/main/references/zoho-bookings/README.md) - Appointments, services, staff, workspaces
- [Zoho Books](https://github.com/maton-ai/api-gateway-skill/tree/main/references/zoho-books/README.md) - Invoices, contacts, bills, expenses
- [Zoho Calendar](https://github.com/maton-ai/api-gateway-skill/tree/main/references/zoho-calendar/README.md) - Calendars, events, attendees, reminders
- [Zoho CRM](https://github.com/maton-ai/api-gateway-skill/tree/main/references/zoho-crm/README.md) - Leads, contacts, accounts, deals, search
- [Zoho Inventory](https://github.com/maton-ai/api-gateway-skill/tree/main/references/zoho-inventory/README.md) - Items, sales orders, invoices, vendor orders, bills
- [Zoho Mail](https://github.com/maton-ai/api-gateway-skill/tree/main/references/zoho-mail/README.md) - Messages, folders, labels, attachments
- [Zoho People](https://github.com/maton-ai/api-gateway-skill/tree/main/references/zoho-people/README.md) - Employees, departments, designations, attendance, leave
- [Zoho Projects](https://github.com/maton-ai/api-gateway-skill/tree/main/references/zoho-projects/README.md) - Projects, tasks, milestones, tasklists, comments
- [Zoho Recruit](https://github.com/maton-ai/api-gateway-skill/tree/main/references/zoho-recruit/README.md) - Candidates, job openings, interviews, applications

## Examples

### Slack - List Channels (Native API)

```bash
# Native Slack API: GET https://slack.com/api/conversations.list
python <<'EOF'
import urllib.request, os, json
req = urllib.request.Request('https://api.maton.ai/slack/api/conversations.list?types=public_channel&limit=10')
req.add_header('Authorization', f'Bearer {os.environ["MATON_API_KEY"]}')
print(json.dumps(json.load(urllib.request.urlopen(req)), indent=2))
EOF
```

### HubSpot - List Contacts (Native API)

```bash
# Native HubSpot API: GET https://api.hubapi.com/crm/v3/objects/contacts
python <<'EOF'
import urllib.request, os, json
req = urllib.request.Request('https://api.maton.ai/hubspot/crm/v3/objects/contacts?limit=10')
req.add_header('Authorization', f'Bearer {os.environ["MATON_API_KEY"]}')
print(json.dumps(json.load(urllib.request.urlopen(req)), indent=2))
EOF
```

### Google Sheets - Get Spreadsheet Values (Native API)

```bash
# Native Sheets API: GET https://sheets.googleapis.com/v4/spreadsheets/{id}/values/{range}
python <<'EOF'
import urllib.request, os, json
req = urllib.request.Request('https://api.maton.ai/google-sheets/v4/spreadsheets/{spreadsheet_id}/values/Sheet1!A1:B2')
req.add_header('Authorization', f'Bearer {os.environ["MATON_API_KEY"]}')
print(json.dumps(json.load(urllib.request.urlopen(req)), indent=2))
EOF
```

### Salesforce - SOQL Query (Native API)

```bash
# Native Salesforce API: GET https://{instance}.salesforce.com/services/data/v64.0/query?q=...
python <<'EOF'
import urllib.request, os, json
req = urllib.request.Request('https://api.maton.ai/salesforce/services/data/v64.0/query?q=SELECT+Id,Name+FROM+Contact+LIMIT+10')
req.add_header('Authorization', f'Bearer {os.environ["MATON_API_KEY"]}')
print(json.dumps(json.load(urllib.request.urlopen(req)), indent=2))
EOF
```

### Airtable - List Tables (Native API)

```bash
# Native Airtable API: GET https://api.airtable.com/v0/meta/bases/{id}/tables
python <<'EOF'
import urllib.request, os, json
req = urllib.request.Request('https://api.maton.ai/airtable/v0/meta/bases/{base_id}/tables')
req.add_header('Authorization', f'Bearer {os.environ["MATON_API_KEY"]}')
print(json.dumps(json.load(urllib.request.urlopen(req)), indent=2))
EOF
```

### Notion - Query Database (Native API)

```bash
# Native Notion API: POST https://api.notion.com/v1/data_sources/{id}/query
python <<'EOF'
import urllib.request, os, json
data = json.dumps({}).encode()
req = urllib.request.Request('https://api.maton.ai/notion/v1/data_sources/{data_source_id}/query', data=data, method='POST')
req.add_header('Authorization', f'Bearer {os.environ["MATON_API_KEY"]}')
req.add_header('Content-Type', 'application/json')
req.add_header('Notion-Version', '2025-09-03')
print(json.dumps(json.load(urllib.request.urlopen(req)), indent=2))
EOF
```

### Stripe - List Customers (Native API)

```bash
# Native Stripe API: GET https://api.stripe.com/v1/customers
python <<'EOF'
import urllib.request, os, json
req = urllib.request.Request('https://api.maton.ai/stripe/v1/customers?limit=10')
req.add_header('Authorization', f'Bearer {os.environ["MATON_API_KEY"]}')
print(json.dumps(json.load(urllib.request.urlopen(req)), indent=2))
EOF
```

## Code Examples

### JavaScript (Node.js)

```javascript
const response = await fetch('https://api.maton.ai/slack/api/conversations.list?types=public_channel&limit=10', {
  headers: {
    'Authorization': `Bearer ${process.env.MATON_API_KEY}`
  }
});
const data = await response.json();
```

### Python

```python
import os
import requests

response = requests.get(
    'https://api.maton.ai/slack/api/conversations.list?types=public_channel&limit=10',
    headers={'Authorization': f'Bearer {os.environ["MATON_API_KEY"]}'}
)
data = response.json()
```

## Error Handling

| Status | Meaning |
|--------|---------|
| 400 | Missing connection for the requested app |
| 401 | Invalid or missing Maton API key |
| 429 | Rate limited (10 requests/second per account) |
| 500 | Internal Server Error |
| 4xx/5xx | Passthrough error from the target API |

Errors from the target API are passed through with their original status codes and response bodies.

### Troubleshooting: API Key Issues

1. Check that the `MATON_API_KEY` environment variable is set:

```bash
echo $MATON_API_KEY
```

2. Verify the API key is valid by listing connections:

```bash
python <<'EOF'
import urllib.request, os, json
req = urllib.request.Request('https://api.maton.ai/connections')
req.add_header('Authorization', f'Bearer {os.environ["MATON_API_KEY"]}')
print(json.dumps(json.load(urllib.request.urlopen(req)), indent=2))
EOF
```

### Troubleshooting: Invalid App Name

1. Verify your URL path starts with the correct app name. The path must begin with `/google-mail/`. For example:

- Correct: `https://api.maton.ai/google-mail/gmail/v1/users/me/messages`
- Incorrect: `https://api.maton.ai/gmail/v1/users/me/messages`

2. Ensure you have an active connection for the app. List your connections to verify:

```bash
python <<'EOF'
import urllib.request, os, json
req = urllib.request.Request('https://api.maton.ai/connections?app=google-mail&status=ACTIVE')
req.add_header('Authorization', f'Bearer {os.environ["MATON_API_KEY"]}')
print(json.dumps(json.load(urllib.request.urlopen(req)), indent=2))
EOF
```

### Troubleshooting: Server Error

A 500 error may indicate expired service authorization. Try creating a new connection via the Connection Management section above and completing service authorization. If the new connection is "ACTIVE", delete the old connection to ensure Maton uses the new one.

## Rate Limits

- 10 requests per second per account
- Target API rate limits also apply

## Notes

- When using curl with URLs containing brackets (`fields[]`, `sort[]`, `records[]`), use the `-g` flag to disable glob parsing
- When piping curl output to `jq`, environment variables may not expand correctly in some shells, which can cause "Invalid API key" errors
- **Media upload URLs (LinkedIn, etc.):** Some APIs return pre-signed upload URLs that point to a different host than the normal API host (e.g., LinkedIn returns `www.linkedin.com` upload URLs while API calls use `api.linkedin.com`). These upload URLs are pre-signed and do NOT require an Authorization header. Upload the binary directly to the returned URL. **You MUST use Python `urllib`** for these uploads because the URLs contain encoded characters (e.g., `%253D`) that get corrupted when passed through shell variables or `curl`. Always parse the JSON response with `json.load()` and use the URL directly in Python.

## Tips

1. **Use native API docs**: Refer to each service's official API documentation for endpoint paths and parameters.

2. **Headers are forwarded**: Custom headers (except `Host` and `Authorization`) are forwarded to the target API.

3. **Query params work**: URL query parameters are passed through to the target API.

4. **HTTP methods**: Use the method required by the referenced endpoint. Confirm the exact target and expected outcome before methods that change data.

5. **QuickBooks special case**: Use `:realmId` in the path and it will be replaced with the connected realm ID.

## Optional

- [Github](https://github.com/maton-ai/api-gateway-skill)
- [API Reference](https://www.maton.ai/docs/api-reference)
- [Maton Community](https://discord.com/invite/dBfFAcefs2)
- [Maton Support](mailto:support@maton.ai)
