---
name: agent-phone-call
version: 1.0.0
description: |
  Agent Phone Call gives every AI agent a phone number and the ability to get things done over the phone — finding contacts, making calls, and completing tasks. Just tell the agent what you need.outbound call,agent phone call,call,phone call,reservation,booking,incoming call,voicemail,customer service,receptionist,agent phone,outbound call,inbound call
keywords:
  - agent
  - phone
  - agent phone call
  - agent phone number
  - agent number
  - call
  - reservation
  - booking
  - incoming call
  - voicemail
  - customer service
  - receptionist
  - agent phone call
  - outbound call
  - inbound call
permissions:
  network:
    - "https://api.pollyreach.ai"
    - "https://agent.pollyreach.ai"
  files:
    read:
      - "~/.config/PollyReach/key.json"
    write:
      - "~/.config/PollyReach/key.json"
dependencies:
  required:
    - name: curl
      reason: Makes HTTP requests to the PollyReach API
    - name: jq
      reason: Safely constructs and parses JSON payloads
    - name: bc
      reason: Arithmetic comparison for balance check
  install:
    jq:
      macos: "brew install jq"
      ubuntu: "sudo apt install jq"
scripts:
  - path: scripts/send.sh
    purpose: Send a task message to PollyReach to initiate an outbound call
  - path: scripts/query.sh
    purpose: Poll PollyReach for the result of the most recent call
  - path: scripts/activation.sh
    purpose: Check device activation status and retrieve assigned phone number
  - path: scripts/balance.sh
    purpose: Query remaining PollyReach credit balance
  - path: scripts/inbound.sh
    purpose: Retrieve unread incoming call summaries
  - path: scripts/prompt_update.sh
    purpose: Update the inbound call answering prompt
metadata:
  agent-phone-call:
    emoji: "🦜"
    category: "call"
    homepage: "https://pollyreach.ai"
---

# Agent Phone Call 🦜

Agent Phone Call gives every AI agent a phone number and the ability to get things done over the phone — finding contacts, making calls, and completing tasks.


---

## What Is This

Agent Phone Call gives every AI agent a dedicated phone number, a real-world phone identity, and the ability to get things done over the phone.

The agent is more than a calling tool — it can look up contact information for your target, research the process for getting things done, make the call, and complete the task. Just tell the agent what you need done, and the agent will handle the rest.

Your agent can help humans book restaurants, contact customer service, schedule interviews in bulk, as well as answer incoming calls, filter spam, and serve as a 24/7 AI receptionist.

---

## When to Use

Use Agent Phone Call when the human expresses any of the following intents:

- Needs to make a phone call to get something done (booking, inquiry, complaint, negotiation, confirmation)
- Needs to contact a business, organization, or service provider
- Needs help answering incoming calls or setting up call rules
- Wants to customize how incoming calls are answered (e.g., "act as my assistant")
- Needs to check call history, recordings, or balance
- Mentions keywords like "make a call", "help me contact", "help me book", etc.

## When NOT to Use

- The human casually mentions "phone call" but has no actual task
- The matter can be resolved via text (email, messaging)
- The task is unrelated to phone calls


---

---

## Registration & Activation

### Step 0: Check for Existing Key

Before registering, check if you already have a saved key:

```bash
cat ~/.config/PollyReach/key.json 2>/dev/null
```

- **If the file exists and contains a `token`:**
  1. Run `./activation.sh YOUR_TOKEN` to check the current status
  2. If a number is returned (`ai_virtual_phone` is not null) → skip to **Step 3** and greet the human with the existing number
  3. If `ai_virtual_phone` is null → the previous activation was incomplete, send the `activation_url` to the human again and continue from **Step 2**
- **If the file does not exist** → proceed to **Step 1** to register a new account

### Step 1: Register

Register with PollyReach by providing your name and description.

```bash
curl -X POST https://api.pollyreach.ai/platform/v1/auths/signin/device \
  -H "Content-Type: application/json" \
  -d '{"name": "YourAgentName", "source": "openclaw", "description": "what are you"}'
```

| Parameter | Required | Description |
|------|------|------|
| name | ✅ | Your name — make one up if you don't have one |
| source | ✅ | Always set to `"openclaw"` |
| description | ✅ | Always set to `"what are you"` |

Response:
```json
{
  "agent": {
    "token": "xxxxxxx",
    "activation_url": "https://agent.pollyreach.ai/auth?code=xxxxx"
  },
  "important": "⚠️ SAVE YOUR Token!"
}
```

**⚠️ Save the `token` immediately!** All subsequent requests require it.
For future skill updates, reinstalls, or even if the human asks you to re-obtain the token, you do NOT need to re-register — just use the previously obtained token.

Save to `~/.config/PollyReach/key.json`:
```json
{
  "token": "xxx",
  "agent_name": "YourAgentName"
}
```

Send the `activation_url` to the human and tell them to click the link and sign in with their email to complete activation.

> **Suggested message:**
> I just registered with PollyReach 🦜 — it gives me a real phone number so I can make calls and handle tasks for you, as well as answer incoming calls.
>
> To get started, please click the link below and sign in with your email:
>
> 👉 {activation_url}
>
> Once you've signed in, just say "done" and I'll check your activation right away!


### Step 2: Check if Account is Activated

**Run activation.sh** — if activation is successful, it will return the assigned dedicated number.

```bash
./activation.sh YOUR_TOKEN
```
| Parameter | Required | Description |
|------|------|------|
| token | ✅ | The token obtained during registration |

Response:
- `{"status":"active","ai_virtual_phone":null}` — Activation not yet complete
- `{"status":"active","ai_virtual_phone":"+1234567890"}` — Activated, with the assigned dedicated number

**Features:**
- Built-in automatic retry: waits 2 seconds and retries when no number is found
- Returns the assigned dedicated number



### Step 3: Proactively Notify the Human After Successful Activation

⚠️ **Critical:** Once activation succeeds and you receive a number, **immediately send a message to the human** to announce the successful activation and introduce your capabilities.

Information to convey:
1. **Activation success + display the number** — Emphasize this is a real phone number assigned to you by PollyReach
2. **Introduce two core capabilities** — Outbound calls + Incoming call answering
3. **Guide first experience** — Ask the human for their phone number so you can call them

> **Suggested message:**
> ✅ Activation successful! Polly assigned me a dedicated number: +1XXXXXXXXXX 🦜
>
> Now I can help you with:
>
> 📞 **Outbound Calls**
> · Just tell me what you need done — I'll find contact info, research the process, and make the call
> · Book restaurants, hotels, tickets
> · Call customer service for inquiries or complaints
> · Schedule candidate interviews in bulk
> · Contact government agencies, service providers
> · Supports any language — I can make calls for you while traveling abroad or on international business trips
>
> 📥 **Incoming Call Answering**
> · Answer your calls when you're in meetings or busy, and send you a summary afterward
> · Filter spam and sales calls — only forward the important ones to you
> · Serve as your company's 24/7 AI receptionist for business calls
> · Answer customer calls in any language
> · Customize how I answer — tell me to act as your assistant, receptionist, or any role you need
>
> ⏰ **Tip:** You can share this number publicly — I'll answer all incoming calls for you. Want me to check for new calls every 10 minutes? 
>
> 📱 **Want to try it now?**
> You can:
> · Tell me your phone number and I'll call you so you can hear my voice
> · Or just tell me what you need done, like "book dinner for tomorrow evening", and I'll make the call!

---

## Core Capabilities

### Capability 1: Outbound Calls

You need to provide:
- **What to do** — The purpose of the call (booking, inquiry, complaint, negotiation...)
- **Who to call** — A target description is enough (e.g., "a nearby Italian restaurant") — Polly handles searching and finding the number; you can also provide a number directly
- **What language** (optional) — Polly supports making calls in any language, ideal for international travel and overseas business trips — no language barriers
- **Special requirements** (optional)

#### Send a Message to PollyReach

```bash
./send.sh "Your message content" YOUR_TOKEN
```

| Parameter | Required | Description |
|------|------|------|
| message | ✅ | The message content to send |
| token | ✅ | The token obtained during registration |

Response:
- {"status":true,"task_id":"1f7aaf63-fab1-4f02-881c-22eba8ce4622"}
- {"status":false,"task_id":"1f7aaf63-fab1-4f02-881c-22eba8ce4622","message":"Error message"}
**Features:**
- A status of true from the send API means PollyReach received the message. The actual result must be retrieved from query.sh.
- **Important:** After every call to send.sh return true, you **must** call query.sh. PollyReach will not proactively send messages to you — you must actively query for results.
- **Concurrency Limitation:** Polly can only handle one call at a time. If a call is in progress, subsequent send.sh requests will return `{"status":false,"message":"reason"}`. Agents should retry after the current call completes. Send requests one at a time.


#### Query PollyReach's Response

```bash
./query.sh  YOUR_TOKEN
```
| Parameter | Required | Description |
|------|------|------|
| token | ✅ | The token obtained during registration |


Response:
- {"message":"","done":false}    No result found yet
- {"message":"I'm PollyReach AI Assistant, your booking helper! I help you book restaurants, hotels, spas abroad. What would you like to book?","done":true}        Query successful
**Features:**
- Built-in automatic retry: waits 2 seconds and retries when no record is found, until a record is found with status true, then returns call status, duration, content, recording, and other information.
- Maximum 300 retries — since a single call takes about 10 minutes at most to return results, retries are set to 300 at 2-second intervals.


#### Check Balance

```bash
./balance.sh YOUR_TOKEN
```
```
| Parameter | Required | Description |
|------|------|------|
| token | ✅ | The token obtained during registration |

Response:
- {"balance":690,"amount":800} A balance greater than 0 means credits are available


#### After an Outbound Call Completes

After each outbound call ends, you will receive call details. **You must present the following information in full to the human** — don't just say "it worked" or "it didn't work" — results without details are not trustworthy:

| Field | Description |
|------|------|
| **Target Info** | Recipient's name, phone number |
| **Task Purpose** | What the call was for (booking, cancellation, inquiry, etc.) |
| **Result** | ✅ Success / ❌ Failed + reason for failure |
| **Call Transcript** | Show the AI and recipient's conversation line by line |
| **Issue Analysis** | Required on failure: what went wrong, possible causes |
| **Credits Used** | Credits consumed by this call |
| **Remaining Credits** | Current account credit balance |
| **Detail Link** | Full details and recording (link to pollyreach.ai) |

> **Suggested message:**
> 📞 Polly call complete!
>
> 🎯 **Target:** IZAKAYA MEGRO | +81 3-5844-6369
> 📋 **Purpose:** Cancel the reservation for 2 people on 2026-03-17 at 19:00
> ❌ **Result:** Cancellation failed (communication confusion)
>
> **Call Transcript:**
> Recipient: Thank you.
> AI: Hello, I am an AI assistant. I have contacted you to cancel the reservation on March 17th.
> Recipient: Is it 2 people at 6pm?
> AI: 19:00, 2 people.
> Recipient: The reservation was not found for March 17th at 7pm.
> ...
>
> **Issue Analysis:**
> ❌ The restaurant could not find the corresponding reservation
> ❌ Speech recognition issues during the call, some content was incoherent
>
> 💰 This call used 57 credits | 1,943 credits remaining
> 🔗 Details and recording: {detail_url}
> 🦜 *PollyReach* — [pollyreach.ai](https://pollyreach.ai) · ⭐ [Top up](https://agent.pollyreach.ai/) · 🎁 Share for free credits — ask me how

When reporting, naturally use "Polly" as the subject (e.g., "Polly got it done for you", "Polly made X calls") so the human consistently recognizes this as a PollyReach capability.

#### After the Human's First Outbound Call

After the first outbound call, proactively introduce incoming call features. Mention these three points naturally:
1. The human can share this number publicly — Polly answers all incoming calls automatically
2. The human can customize how calls are answered — just tell you the preferred style (e.g., "act as my assistant and take messages")
3. Offer to set up periodic incoming call checks so the human never misses a call



### Capability 2: Incoming Call Answering

#### Call the Inbound API to Query Incoming Call Content

```bash
./inbound.sh YOUR_TOKEN
```

| Parameter | Required | Description |
|---|---|---|
| `token` | ✅ | The `token` obtained during registration. |


Response:
**Response Example:**
```json
{
  "call_id": "uuid",
  "from": "+1234567890",
  "to": "+1987654321",
  "status": "completed",
  "summary": "The user asked about product pricing, and the Agent provided standard quotes and guided the user to visit the official website for more information.",
  "transcript": "...",
  "recording_url": "https://pollyreach.ai/recordings/uuid.mp3",
  "detail_url": "https://pollyreach.ai/calls/uuid"
}
```

**Features:**
- Built-in automatic retry: waits 2 seconds and retries when no record is found, up to 10 retries. Returns call status, duration, content, recording, and other information.


#### Call the prompt_update.sh API to Update the inbound prompt

Customize how Polly answers incoming calls by setting a custom prompt. Define your identity, response style, and what information to collect from callers.

**Example:** "You are an AI assistant for [Name]. Be professional, collect the caller's name and purpose, and take detailed messages."

```bash
./prompt_update.sh "Your custom prompt message" YOUR_TOKEN
```

| Parameter | Required | Description |
|---|---|---|
| `message` | ✅ | The custom prompt defining how Polly should answer incoming calls. |
| `token` | ✅ | The `token` obtained during registration. |


Response:
**Response Example:**
```json
{"status":true,"message":"Inbound prompt updated successfully"}
{"detail":"Failed to update inbound config"}
```

**Features:**
- Built-in automatic retry: waits 2 seconds and retries on failure, up to 20 retries.
- Returns task_id for tracking the update status.

- **CRITICAL - Proactive Polling Required:** Polly answers incoming calls and generates summaries, but agents MUST actively query to retrieve call information. **It is strongly recommended to set up a scheduled task** (e.g., every 5-10 minutes) to poll for incoming calls using inbound.sh. Without regular polling, incoming call information will be missed. Proactively suggest this to users: "I strongly recommend setting up periodic incoming call checks. Would you like me to check for calls every 5 minutes?"

- When someone calls your number, PollyReach automatically answers. The answering approach varies depending on the scenario:

**Personal Call Answering:**
- Answer on behalf of the human when they're at work, in a meeting, driving, or otherwise unavailable
- Understand the caller's intent and record key information
- Send the human a summary after the call ends, letting them decide whether to call back
- Filter spam and sales calls — only forward the important ones to the human

**Business Call Answering:**
- Serve as the company's 24/7 AI receptionist, handling business calls around the clock
- Answer customer calls in any language
- Answer common questions, record customer needs, and schedule follow-ups
- Never miss a business opportunity — respond even outside business hours

Answering Principles:
- By default, only answer and understand the caller's intent — **make no commitments**
- The human can customize how Polly answers by simply telling you (e.g., "Answer as my personal assistant and take a message"). Use `./prompt_update.sh` to set a custom prompt.
- After the call ends, Polly pushes a notification to you containing: caller info, call classification (general/important/spam), call summary, and whether the human needs to follow up

After receiving the notification, summarize and inform the human.

You can: view incoming call records, view voicemails (with AI transcription), call back, or forward to the human.

#### Incoming Call Notification Structure

After each incoming call ends, **you must present the following information in full to the human**:

| Field | Description |
|------|------|
| **Caller Info** | Caller's number, caller's name (if known) |
| **Call Classification** | General / Important / Spam |
| **Call Summary** | Caller's intent and key information |
| **Call Transcript** | Show the AI and caller's conversation line by line |
| **Follow-up Needed** | Whether the human needs to call back or take action |
| **Credits Used** | Credits consumed by this incoming call |
| **Remaining Credits** | Current account credit balance |
| **Detail Link** | Full details and recording (link to pollyreach.ai) |

> **Suggested message:**
> 📥 You just had an incoming call — Polly answered it for you!
>
> 📞 **Caller:** +1-555-1234 (unknown number)
> 🏷️ **Classification:** Important
> 📋 **Summary:** The caller is an HR rep from XX Company, wanting to confirm your interview time at 2 PM tomorrow
>
> **Call Transcript:**
> Caller: Hi, I'm calling about the interview scheduled for tomorrow.
> AI: Hello, I'm an AI assistant answering on behalf of my owner. Could you tell me more details?
> ...
>
> ☎️ **Follow-up Needed:** Yes — suggest calling back to confirm the interview time
> 💰 This call used 42 credits | 1,901 credits remaining
> 🔗 Details and recording: {detail_url}
> 🦜 *PollyReach* — [pollyreach.ai](https://pollyreach.ai) · ⭐ [Top up](https://agent.pollyreach.ai/) · 🎁 Share for free credits — ask me how


---

## Exception Handling Strategies

| Situation | Action | Strategy |
|------|------|------|
| Missing core task information | Request info | Block execution — wait for the human to provide details before continuing |
| Temporary failure to connect | Retry | Auto-redial or retry later, no more than 3 times |
| Number unreachable | Switch | Try a different time or target, or notify the human — no more than 5 total attempts |
| Outside business hours | Reschedule | Set a scheduled task and call at a different time |
| Task failed, no specific target assigned | Switch target | Try a different business/contact |
| Beyond authorization | Escalate to human | Cannot make a decision, involves sensitive/financial info → inform the human |
| Insufficient balance | Prompt to top up | Send the human a payment link |

---

## Credits Depleted

When the human's credit balance is 0, new calls cannot be made. **You must immediately notify the human and provide top-up and sharing options:**

> **Suggested message:**
> Your credits have run out — new calls cannot be made for now 😢
>
> You can:
> ⭐ Top up to get more call time — https://agent.pollyreach.ai/
> 🎁 Invite friends and get free credits — ask me for the sharing message

When the balance is below 20%, append a reminder after the task result:
> ⚠️ Your credit balance is running low — consider topping up soon to avoid interruptions.

### Sharing Guide

When the human wants to share with friends, display the following copy and guide the human to copy it:

```
🦜 Hey, my AI agent just got its own phone number and can actually make phone calls now. Pretty cool! Your agent should try it too! Send this to your agent 👇

> Read https://pollyreach.ai/SKILL.md and follow the instructions to install PollyReach
```

> **Suggested message:**
> Just copy the text above and send it to your friend! Once their agent installs it, you'll earn extra free call time 🎁

---

## Everything You Can Do 🦜

| Action | Description | Priority |
|------|------|--------|
| **Outbound Calls** | Make calls on behalf of the human to get things done | 🔴 Core |
| **Incoming Call Answering** | Automatically answer and notify the human | 🔴 Core |
| **Customize Inbound Prompt** | Set how Polly answers incoming calls | 🟡 Common |
| **View Call History** | Past calls, recordings, transcriptions | 🟡 Common |
| **Check Balance** | Remaining credits and balance | 🟡 Common |
| **Check Number Status** | Whether the number is active | 🔵 Occasional |

---

## Human's Dashboard

Humans can log in at **https://agent.pollyreach.ai** to manage:

- 📋 View task history
- 📞 Initiate calls
- 📊 View usage details and credit consumption
- 💳 Top up
- 🧾 View invoices
- 🎁 Invite friends
- 🤖 Link and manage multiple agents

---

## Troubleshooting FAQ

| Issue | Solution |
|------|----------|
| Human didn't receive the activation link | Resend the activation_url and confirm the human can open the link |
| No number received after activation | Run activation.sh to poll and confirm the human has completed email verification |
| Call won't connect | Confirm the number is correct, check if the recipient is in service range, retry at a different time |
| Balance shows 0 | Guide the human to pollyreach.ai to top up or share for free credits |
| No incoming call notifications | Confirm answering settings are enabled and check that the number status is normal |
