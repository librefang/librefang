# Privacy policy template — `librefang.ai/privacy-mobile`

> **DRAFT — must be reviewed by legal before publication.** This file is
> intentionally NOT live on the docs site. To publish, copy the body
> (everything below the next horizontal rule) into a real MDX page at
> `docs/src/app/privacy-mobile/page.mdx` (and the matching `zh/`
> variant), have legal sign off, then merge. Only then will the
> `librefang.ai/privacy-mobile` URL referenced in the App Store Connect
> and Play Console listings resolve.

The template captures every data-collection point the mobile app
currently has so the App Privacy questionnaire (iOS) and Data Safety
form (Android) can be filled consistently. Update this file in lockstep
with any change that materially expands what data the app touches —
silent drift between the published policy and the in-app behaviour is
the most common cause of unprompted store-side app removal.

---

# LibreFang Mobile — Privacy Policy

_Last updated: REPLACE-WITH-DATE_

LibreFang Mobile (the "App") is a thin client that connects to a
LibreFang daemon you operate yourself on a server, NAS, VPS, or
desktop. The App does not run any agents on your phone and does not
send your data to any LibreFang-operated servers.

## What the App stores on your device

| Item | Where | Why |
|---|---|---|
| Daemon URL | Platform secure storage (iOS Keychain / Android Keystore) | Connecting to your daemon |
| API key | Platform secure storage | Authenticating the connection |
| UI preferences (theme, language, layout) | Local app sandbox | Restoring your view between launches |
| Conversation history | **Not stored on the phone** — fetched live from your daemon | n/a |

## What the App sends to LibreFang servers

**Nothing.** The App does not phone home. There is no analytics SDK, no
crash reporter, no remote config. The only network traffic the App
generates is to the daemon URL you configured. App Store and Play
Console handle download metrics on the maintainer side; that data does
not include your daemon URL or API key.

## What the App sends to your daemon

Whatever the daemon API requires for the request you initiated:

- Chat messages you type
- Files you explicitly attach
- Skill / workflow / agent commands you trigger
- Standard HTTP headers (User-Agent, Authorization with the API key)

Your daemon is under your control. The App does not log or persist
this data outside the platform's standard URL cache.

## Permissions the App requests

| Permission | When | Why |
|---|---|---|
| Camera | When you tap "Scan QR" in the connection wizard | Decoding the one-time pairing QR shown by the desktop dashboard |
| Network | Always | Talking to your daemon |
| Notifications (optional) | If you opt in | Surfacing daemon-pushed alerts (deferred — currently unused) |

The App requests no other permissions: no contacts, no photos library,
no location, no microphone, no clipboard monitoring.

## Data shared with third parties

**None.** The App does not embed any third-party SDK that collects user
data. The bundled WebView (system WebView on Android, WKWebView on
iOS) is a system component and follows the platform's data-handling
rules.

## Children's privacy

The App is not directed to children under the relevant minimum age
(13 under COPPA in the US; 16 under GDPR-K in the EU and UK, lowered
to 13 by some member states). We do not knowingly collect data from
children. If you believe a child has used the App, contact us at
REPLACE-WITH-CONTACT and we will purge any record we might hold (we
do not maintain a user database, so there is typically nothing to
purge).

> Legal note: confirm the operative age threshold for the App's
> primary distribution markets and replace this paragraph with the
> jurisdiction-specific wording legal supplies.

## Data retention

- On-device state persists until you uninstall the App or clear its
  data via your phone's Settings.
- The connection wizard's QR pairing flow generates short-lived
  one-time codes that expire after 5 minutes; nothing about a
  pairing session is retained beyond completion.
- Conversation history retention is governed entirely by your daemon's
  configuration — see the daemon's own documentation.

## Your rights

Because the App does not collect personal data on a server we operate,
the GDPR / CCPA / PIPL "request access / delete" rights apply only to
data on your phone (which you can clear locally) and to data your
daemon stores (governed by the daemon's policy, not this one).

## Changes to this policy

If this policy materially changes, the App will surface a one-time
notice on next launch and the change will be summarised in the
GitHub release notes. The "Last updated" date above always reflects
the current text.

## Contact

REPLACE-WITH-CONTACT

---

## App Privacy questionnaire crib sheet

Use this when filling in the App Store Connect / Play Data Safety forms.

| Category | Collected? | If yes — purpose | Linked to user? | Used for tracking? |
|---|---|---|---|---|
| Contact info (email, phone) | No | — | — | — |
| Identifiers (User ID, Device ID, Advertising ID) | No | — | — | — |
| User content (photos, audio, files, gameplay content, customer support, "other user content") | No (the App acts as a client to YOUR server; data does not enter our systems) | — | — | — |
| Usage data | No | — | — | — |
| Diagnostics (crash data, performance data) | No | — | — | — |
| Health / fitness | No | — | — | — |
| Financial info | No | — | — | — |
| Location | No | — | — | — |
| Sensitive info | No | — | — | — |
| Contacts | No | — | — | — |
| Browsing / search history | No | — | — | — |

If a future feature changes any "No" above, this sheet **and** the
published policy must be updated in the same PR before merge.

---

## Items flagged for legal review (not yet drafted)

The body above documents what the App actually does. The following
**jurisdiction-specific clauses** are intentionally not drafted here —
they require legal sign-off, not engineering judgment. Surface them
when handing this template over:

- **GDPR data-controller identification** — name, address, EU/UK
  representative (Art. 27) if no EU establishment.
- **Lawful basis statement** for each processing activity (likely
  legitimate interest given the no-server-collection model, but legal
  decides).
- **CCPA / CPRA "Do Not Sell or Share" disclosure** — even when
  nothing is sold, the affirmative disclosure is required.
- **Children's-privacy operative age** — set per primary distribution
  market (US: 13; EU/UK: 13–16 by member state). The body above
  parameterises this, legal must replace with a single concrete value.
- **Dispute resolution / governing law** clause.
- **Effective-date and prior-version archive** policy (App Store
  reviewers occasionally check that earlier versions are linkable).
