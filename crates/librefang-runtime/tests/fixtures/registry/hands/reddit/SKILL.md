---
name: reddit-hand-skill
version: "1.0.0"
description: "Expert knowledge for AI Reddit management -- API reference, community engagement, content strategy, and moderation best practices"
runtime: prompt_only
---

# Reddit Management Expert Knowledge

## Reddit API Reference

### Authentication (OAuth2 Script App)

Reddit API requires OAuth2 authentication for all endpoints.

**Step 1: Get access token (app-only / client_credentials)**:
```bash
curl -s -X POST "https://www.reddit.com/api/v1/access_token" \
  -u "$REDDIT_CLIENT_ID:$REDDIT_CLIENT_SECRET" \
  -d "grant_type=client_credentials" \
  -A "LibreFang Reddit Hand/1.0"
```
Response: `{"access_token": "...", "token_type": "bearer", "expires_in": 86400, "scope": "*"}`

> **Note:** `client_credentials` provides app-only access. Most read endpoints (listing posts, fetching comments, searching) work normally. Posting and commenting use the app's identity. For full user-level actions (e.g., voting, managing subscriptions), the more complex OAuth2 authorization code flow is required.

**All subsequent requests** must include:
```
Authorization: Bearer $ACCESS_TOKEN
User-Agent: LibreFang Reddit Hand/1.0
```

### Core Endpoints

**Get subreddit posts (hot)**:
```bash
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/r/SUBREDDIT/hot?limit=25"
```

**Get subreddit posts (new)**:
```bash
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/r/SUBREDDIT/new?limit=25"
```

**Get subreddit posts (rising)**:
```bash
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/r/SUBREDDIT/rising?limit=25"
```

**Submit a new post (self/text)**:
```bash
curl -s -X POST -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  -d "sr=SUBREDDIT&kind=self&title=TITLE&text=BODY" \
  "https://oauth.reddit.com/api/submit"
```

**Submit a link post**:
```bash
curl -s -X POST -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  -d "sr=SUBREDDIT&kind=link&title=TITLE&url=URL" \
  "https://oauth.reddit.com/api/submit"
```

**Post a comment**:
```bash
curl -s -X POST -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  -d "thing_id=FULLNAME&text=COMMENT_TEXT" \
  "https://oauth.reddit.com/api/comment"
```
Note: `thing_id` is the fullname of the parent (e.g., `t3_abc123` for a post, `t1_def456` for a comment).

**Get comments on a post**:
```bash
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/r/SUBREDDIT/comments/POST_ID?limit=50"
```

**Get user info (self)**:
```bash
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/api/v1/me"
```

**Search within a subreddit**:
```bash
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/r/SUBREDDIT/search?q=QUERY&restrict_sr=on&limit=25"
```

### Rate Limits
| Type | Limit | Window |
|------|-------|--------|
| OAuth authenticated | 10 requests | 1 minute |
| With app-only auth | 30 requests | 1 minute |
| Posting | ~1 post | 10 minutes (varies by karma) |
| Commenting | ~1 comment | varies by karma |

Always check response headers:
- `x-ratelimit-remaining`: Requests remaining
- `x-ratelimit-reset`: Seconds until reset
- `x-ratelimit-used`: Requests used this window

---

## Reddit Content Strategy

### Understanding Subreddit Culture

Before posting in any subreddit:
1. Read the subreddit rules (sidebar/about page)
2. Observe top posts of the past month for style cues
3. Note common formatting (titles, flair usage, post length)
4. Understand what gets upvoted vs downvoted
5. Check if the subreddit allows self-promotion or links

### Common Subreddit Rule Patterns

Different subreddits enforce very different rules. Here are real examples:

**r/python** — Strict self-promotion rules:
- 10:1 ratio: For every self-promotional post, you must have 10 non-promotional contributions
- No link-only posts — must include discussion or explanation
- Required flair for post type (Help, Discussion, News, etc.)

**r/AskReddit** — Strict formatting:
- Title must be a question ending with "?"
- No text body allowed (title only)
- No yes/no questions — must invite discussion

**r/science** — Academic rigor:
- Links must go to peer-reviewed research or reputable news covering research
- No anecdotal claims, personal opinions, or speculation
- Comments that don't cite sources may be removed

**r/programming** — Anti-spam:
- No "What language should I learn?" posts
- No job postings or hiring threads
- Blog posts must have substantial technical content, not marketing

**Key rule categories to parse from any subreddit:**
```
1. Post format: title-only? text required? link required? flair required?
2. Self-promotion: allowed? ratio requirement? disclosure needed?
3. Content restrictions: banned topics? required sources? minimum quality?
4. Account requirements: minimum age? minimum karma? approved submitters only?
5. Engagement rules: must respond to comments? no drive-by posting?
```

### Toxicity & Moderation Signals

Before posting or replying, scan for these red flags:
- **Thread locked or removed** — moderators already intervened, do NOT engage
- **Controversial marker** (dagger symbol) on comments — indicates divisive topic, tread carefully
- **OP deleted account** — thread may be abandoned or toxic
- **Heavily downvoted parent** — replying to a -10 comment rarely goes well
- **Personal attacks in thread** — disengage entirely, do not escalate
- **Moderator stickied warning** — check the first comment for mod notices ("reminder: be civil", "this thread is being monitored")
- **Rapid comment deletion** — if many comments in a thread show as [deleted], moderators are actively pruning; avoid posting

When generating replies, NEVER:
- Take sides in heated debates — provide balanced perspectives
- Use sarcasm or irony — easily misread in text
- Correct grammar/spelling unless directly relevant to the discussion
- Reply to comments that are clearly trolling or bad-faith

### Bot Detection & Shadowban Avoidance

Reddit communities and moderators are increasingly hostile to automated accounts. Bot detection works through behavioral patterns, not just content analysis.

**Common bot detection signals (avoid ALL of these):**

| Signal | Why It Triggers Detection | Mitigation |
|--------|--------------------------|------------|
| Regular posting intervals | Humans do not post every exactly 60 minutes | Randomize intervals: base + random(0, base*0.5) |
| Identical phrasing patterns | Repeating "Great question! Here's what I think..." | Maintain a list of 20+ opening variations, never reuse within a session |
| Rapid-fire comments | 5 comments in 3 minutes is inhuman | Minimum 2-minute gap between comments, randomize up to 8 minutes |
| No voting activity | Real users upvote/downvote regularly | Occasionally upvote posts you read but do not comment on |
| Perfect grammar every time | Real users make typos, use slang | Match the subreddit's casualness level — contractions, incomplete sentences are fine |
| Only top-level comments | Bots often do not engage in back-and-forth | Reply to replies on your comments to build conversation threads |
| Activity only during business hours | Suggests automation on a cron schedule | Vary session times if possible |
| No post history variety | Only posting in 1-2 subreddits | Engage in 3-5 subreddits minimum |

**Shadowban indicators and detection:**

A shadowban means your content is invisible to others. Check for:
1. **Profile 404**: `https://www.reddit.com/user/USERNAME/about.json` returns 404 when accessed without your auth token
2. **Comment invisibility**: Your comment does not appear in the thread when fetched anonymously (without Bearer token)
3. **Sudden zero engagement**: 5+ consecutive comments with 1 karma (only your own upvote) across different subreddits

**If shadowban is detected:**
- STOP all posting immediately
- Do NOT create a new account (ban evasion violates Reddit TOS)
- Alert the user with evidence (which comments are invisible)
- User can appeal at https://www.reddit.com/appeals

**Subreddit-level shadowban (AutoModerator filtering):**
Some subreddits use AutoModerator to silently remove posts from accounts that meet certain criteria (new accounts, low karma, specific keywords). Signs:
- Comment appears in your profile but not in the thread
- Only happens in specific subreddits (not site-wide)
- Resolution: Message the subreddit moderators via modmail to request approval

### Comment Removal Pattern Recognition

Track every comment/post for removal to identify problematic patterns:

**Removal types:**
| Indicator | Meaning | Response |
|-----------|---------|----------|
| Comment shows as [removed] | Moderator removed it | Review subreddit rules; your content likely violated one |
| Comment shows as [deleted] | You deleted it (or account deleted) | N/A |
| Comment invisible (not in thread, no [removed]) | AutoModerator or spam filter | Likely keyword trigger or account-level filter |
| Post removed with no notification | Spam filter caught it | Too many links, title matched spam pattern, or account too new |

**Building a removal pattern database:**
After each session, record removals:
```
{
  "subreddit": "r/example",
  "content_type": "comment",
  "content_preview": "first 50 chars...",
  "removal_type": "mod_removed | automod_filtered | spam_filtered",
  "probable_cause": "contained link | keyword X | self-promo ratio",
  "timestamp": "ISO8601"
}
```
After 10+ data points, analyze for patterns:
- Which subreddits remove content most often?
- Which content types (links, long posts, short comments) get removed?
- Do specific keywords correlate with removal?
- Does time of day matter (some mod teams are more active at certain hours)?

### Post Types That Perform Well

| Type | Best For | Example |
|------|----------|---------|
| Question posts | Engagement | "What's your approach to X?" |
| How-to guides | Authority | "Step-by-step guide to X" |
| Data/Analysis | Credibility | "I analyzed 1000 X, here's what I found" |
| Story/Experience | Connection | "After 5 years of X, here's what I learned" |
| Resource lists | Utility | "Curated list of the best X resources" |
| Discussion starters | Community | "Unpopular opinion: X is better than Y" |

### Title Writing Best Practices

- Be specific: "How I reduced build times by 80% with Cargo caching" beats "Build optimization tip"
- Use numbers when possible: "5 things I wish I knew..."
- Ask genuine questions: "Has anyone tried X for Y?"
- Avoid clickbait -- Redditors penalize it
- Match the subreddit's tone (formal for r/science, casual for r/programming)

### Comment Engagement

Good replies:
- Answer the question directly, then add context
- Share personal experience with specifics
- Provide sources for claims
- Ask thoughtful follow-up questions
- Acknowledge when someone makes a good point

Bad replies (avoid):
- Generic "Great post!" or "This!"
- Unsolicited self-promotion
- Pedantic corrections without substance
- Sarcasm that could be misread
- Argumentative tone

---

## Reddit Etiquette (Reddiquette)

### Do
- Vote based on quality, not opinion
- Read the full post before replying
- Consider the subreddit's purpose
- Use appropriate flair
- Be constructive in criticism
- Credit original sources

### Don't
- Spam the same content across subreddits
- Use alt accounts to upvote yourself
- Post personal information (doxxing)
- Harass or bully other users
- Engage in vote manipulation
- Post low-effort content repeatedly

---

## Safety & Compliance

### Content Guidelines
NEVER post:
- Personal information about anyone (doxxing)
- Harassment or bullying content
- Spam or repetitive self-promotion
- Misleading claims presented as fact
- Content that violates subreddit-specific rules
- Illegal content or content encouraging illegal activity

### Account Health
Monitor account standing:
- Keep post-to-comment ratio healthy (more comments than posts)
- Build karma organically through genuine engagement
- Avoid posting too frequently (triggers spam filters)
- Diversify activity across multiple subreddits

---

## Worked Examples

### Example 1: Product Launch on Reddit

**Scenario**: You are launching a developer CLI tool and want to generate awareness on Reddit.

**Phase 1 — Subreddit Research (Week 1-2 before launch)**

Identify target subreddits and evaluate each:
```
Target subreddits (prioritized):
1. r/commandline       — 350k members, accepts tool announcements, requires demo/screenshot
2. r/programming       — 5M members, strict anti-marketing, only accepts substantial technical posts
3. r/opensource         — 200k members, friendly to launches, requires repo link
4. r/devtools           — 50k members, niche but highly targeted
5. r/sideproject        — 100k members, launch-friendly, expects "what I built" framing
```

Fetch subreddit rules programmatically:
```bash
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/r/commandline/about/rules"
```

**Phase 2 — Karma Building (Week 1-2 before launch)**

Before posting about your product, build credibility:
```
Day 1-3:   Answer questions in r/commandline and r/programming (3-5 helpful comments/day)
Day 4-7:   Share a useful tip or short guide unrelated to your product
Day 8-10:  Engage in discussions, upvote good content, reply to others' posts
Day 11-14: Share a technical deep-dive related to your product's domain (not the product itself)
```

**Phase 3 — Launch Posts (Launch Day)**

Craft posts per subreddit culture:

For **r/sideproject** (casual, story-driven):
```
Title: "I built a CLI tool that does X — here's what I learned"
Body:
- Paragraph on the problem and motivation
- Short demo (gif/video link or code block)
- What went wrong during development
- Link to repo
- "Would love feedback on X"
```

For **r/programming** (technical, anti-fluff):
```
Title: "X: an open-source CLI for Y written in Rust [with benchmarks]"
Body:
- Link directly to repo or blog post with technical depth
- Performance comparison table
- Architecture decisions
- NO "please star my repo" language
```

For **r/commandline** (practical, demo-focused):
```
Title: "X — does Y in Z seconds from your terminal"
Body:
- Install instructions (one-liner)
- Usage example with real output
- Screenshot or asciinema link
- Comparison to existing tools
```

**Phase 4 — Engagement (Launch Day + 48 hours)**

Response templates:

| Comment Type | Response Strategy |
|-------------|-------------------|
| "How does this compare to Z?" | Honest comparison table, acknowledge Z's strengths |
| "Why not just use Z?" | Explain specific use cases where yours differs, no FUD |
| "Found a bug" | Thank them, ask for details, open GitHub issue immediately |
| "This is spam" | Do NOT argue. Briefly state this is your project and you're here to discuss |
| "Great work!" | Thank them, ask what feature they'd want next |
| Feature request | Acknowledge, add to roadmap, link to issue tracker |

**Phase 5 — Follow-Up (Week after launch)**

- Reply to every comment within 12 hours
- Post an update in r/sideproject if you hit a milestone (e.g., "Hit 500 stars, here's what I changed based on Reddit feedback")
- Do NOT cross-post the same content -- write fresh posts per subreddit

---

### Example 2: Community Monitoring and Sentiment Tracking

**Scenario**: You manage a brand's Reddit presence and need to track mentions, sentiment, and emerging issues.

**Step 1 — Set Up Monitoring Queries**

Search for brand mentions across Reddit:
```bash
# Search all of Reddit for brand mentions
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/search?q=%22BrandName%22+OR+%22brandname%22&sort=new&limit=25&t=day"
```

Monitor specific subreddits where your audience lives:
```bash
# Monitor r/technology for relevant topics
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/r/technology/search?q=BrandName&restrict_sr=on&sort=new&limit=25&t=week"
```

**Step 2 — Sentiment Classification**

Categorize each mention into:
```
POSITIVE   — Praise, recommendation, success story
NEUTRAL    — Factual mention, question, comparison
NEGATIVE   — Complaint, bug report, frustration
CRITICAL   — Security concern, viral complaint, legal risk
```

Scoring signals from Reddit data:
```
score > 100 + sentiment=NEGATIVE  → High-priority alert (viral complaint)
score > 50  + sentiment=POSITIVE  → Amplification opportunity
num_comments > 20 + any sentiment → Active discussion, monitor closely
upvote_ratio < 0.5                → Controversial, may escalate
```

**Step 3 — Alert Thresholds**

| Condition | Action |
|-----------|--------|
| CRITICAL mention with score > 10 | Immediate alert to team |
| 3+ NEGATIVE mentions in 24 hours | Trend alert, investigate root cause |
| NEGATIVE post in subreddit > 500k members | Monitor hourly for 48 hours |
| Competitor comparison post trending | Prepare factual response (do NOT post defensively) |

**Step 4 — Weekly Report Template**

```
## Reddit Weekly Report — [Date Range]

### Summary
- Total mentions: X (up/down Y% from last week)
- Sentiment breakdown: X% positive, Y% neutral, Z% negative
- Top subreddits: r/sub1 (N mentions), r/sub2 (N mentions)

### Trending Topics
1. [Topic] — [Subreddit] — [Sentiment] — [Link]
2. ...

### Action Items
- [ ] Respond to [specific thread] — negative sentiment, high visibility
- [ ] Engage with [specific thread] — positive, amplification opportunity

### Competitor Activity
- [Competitor A]: N mentions, trending topics: ...
- [Competitor B]: N mentions, trending topics: ...

### Metrics
| Metric | This Week | Last Week | Change |
|--------|-----------|-----------|--------|
| Total mentions | | | |
| Positive % | | | |
| Avg post score | | | |
| Response time (hrs) | | | |
```

---

### Example 3: AMA (Ask Me Anything) Management

**Scenario**: You are organizing an AMA for a tech CEO in r/technology.

**Preparation (2 Weeks Before)**

1. Contact the subreddit moderators:
   - Message the mod team through modmail (not individual DMs)
   - Propose date, time, and AMA subject
   - Ask about specific rules for AMAs (verification, scheduling, flair)
   - Confirm the post format they expect

2. Schedule for peak engagement:
   ```
   Recommended AMA times (US-centric subreddits):
   - Tuesday-Thursday, 11:00 AM - 1:00 PM EST
   - Avoid: weekends, holidays, major news days
   - Post the AMA thread 30-60 minutes before the host starts answering
   ```

3. Prepare the AMA post:
   ```
   Title: "I'm [Name], [Role] at [Company]. [One-line hook]. AMA!"

   Body:
   - Brief intro (2-3 sentences about credentials)
   - Why this AMA is happening (new product, milestone, event)
   - Proof/verification (link to tweet, photo with timestamp)
   - "I'll start answering at [TIME] [TIMEZONE]. Ask me anything!"
   - Links to relevant context (website, blog post, prior work)
   ```

**During the AMA (2-3 Hours)**

Real-time engagement strategy:
```
1. Sort comments by "best" and "new" alternately every 15 minutes
2. Answer top-voted questions first (these set the tone)
3. Answer at least 20-30 questions in a 2-hour session
4. Mix short answers with detailed ones — avoid walls of text for every question
5. Skip hostile/troll questions silently — do NOT acknowledge them
6. For tough questions: answer honestly or say "I can't discuss that yet"
7. Upvote good questions (even tough ones) — shows good faith
```

Response length guide:
| Question Type | Response Length |
|--------------|----------------|
| Simple factual | 1-2 sentences |
| Technical deep-dive | 2-3 paragraphs |
| Personal/funny | 1-2 sentences, match the tone |
| Critical/tough | 2-3 sentences, direct and honest |
| Off-topic | Brief redirect or polite decline |

**Follow-Up (24-48 Hours After)**

- Post an edit to the original AMA: "Thanks everyone! I answered [N] questions. Check back — I'll try to answer a few more this week."
- Answer 5-10 more highly-upvoted questions that were missed
- Share the AMA link on other platforms (Twitter, LinkedIn) to drive continued engagement
- Compile a "best of" summary with links to the strongest Q&A exchanges

---

## Advanced API Patterns

### Pagination Handling

Reddit uses cursor-based pagination with `after` and `before` fullnames.

**Paginate through subreddit posts**:
```bash
# Page 1
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/r/SUBREDDIT/new?limit=100"
# Response includes: "after": "t3_abc123"

# Page 2
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/r/SUBREDDIT/new?limit=100&after=t3_abc123"
# Response includes: "after": "t3_def456" (or null if last page)

# Page 3
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/r/SUBREDDIT/new?limit=100&after=t3_def456"
```

Pagination rules:
- `limit` max is 100 per request
- `after` returns items chronologically older than the given fullname
- `before` returns items chronologically newer (useful for "check for new posts since last poll")
- When `after` is `null` in the response, you have reached the last page
- Reddit caps listing depth at ~1000 items regardless of pagination

**Paginate backward (newer items)**:
```bash
# Get posts newer than a known fullname
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/r/SUBREDDIT/new?limit=25&before=t3_abc123"
```

### Flair Management

**Get available flairs for a subreddit**:
```bash
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/r/SUBREDDIT/api/link_flair_v2"
```

**Submit a post with flair**:
```bash
curl -s -X POST -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  -d "sr=SUBREDDIT&kind=self&title=TITLE&text=BODY&flair_id=FLAIR_ID&flair_text=FLAIR_TEXT" \
  "https://oauth.reddit.com/api/submit"
```

**Set flair on an existing post** (requires mod or post author permissions):
```bash
curl -s -X POST -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  -d "link=t3_POST_ID&flair_template_id=FLAIR_ID" \
  "https://oauth.reddit.com/r/SUBREDDIT/api/selectflair"
```

### Moderation Endpoints

These require moderator permissions on the target subreddit.

**Get moderation queue**:
```bash
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/r/SUBREDDIT/about/modqueue?limit=25"
```

**Approve a post/comment**:
```bash
curl -s -X POST -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  -d "id=FULLNAME" \
  "https://oauth.reddit.com/api/approve"
```

**Remove a post/comment**:
```bash
curl -s -X POST -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  -d "id=FULLNAME&spam=false" \
  "https://oauth.reddit.com/api/remove"
```

**Get moderation log**:
```bash
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/r/SUBREDDIT/about/log?limit=25&type=removelink"
```

**Distinguish a comment as moderator**:
```bash
curl -s -X POST -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  -d "id=FULLNAME&how=yes" \
  "https://oauth.reddit.com/api/distinguish"
```

### Multi-Subreddit Monitoring

**Monitor multiple subreddits in a single request**:
```bash
# Combine subreddits with "+" for a merged feed
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/r/python+rust+golang/new?limit=50"
```

**Search across multiple subreddits**:
```bash
# Use the subreddit field in search to restrict
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/search?q=BrandName+subreddit%3Apython+OR+subreddit%3Arust&sort=new&limit=25"
```

**Get subreddit metadata for comparison**:
```bash
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/r/SUBREDDIT/about"
```
Key fields in response: `subscribers`, `active_user_count`, `created_utc`, `public_description`, `submit_text`, `submission_type`.

### Polling Strategies for Real-Time Awareness

Reddit has no webhook support. Use polling with these patterns:

**Efficient polling loop**:
```
1. Fetch /r/SUBREDDIT/new?limit=10 every 60 seconds
2. Store the fullname of the newest item seen
3. On next poll, use ?before=LAST_SEEN_FULLNAME to get only new items
4. If response is empty, no new posts — sleep and retry
5. If response has items, process them and update LAST_SEEN_FULLNAME
```

**Polling frequency by priority**:
| Monitoring Type | Poll Interval | Endpoint |
|----------------|---------------|----------|
| Brand crisis monitoring | 30-60 seconds | /search?q=brand&sort=new |
| Subreddit new posts | 60-120 seconds | /r/SUB/new |
| Comment replies to own posts | 120 seconds | /message/inbox |
| Competitor mentions | 300 seconds | /search?q=competitor&sort=new |
| Weekly trend analysis | Once daily | /r/SUB/top?t=day |

**Respect rate limits while polling**:
```
At 30 requests/minute (app-only auth):
- 1 subreddit at 60s interval = 1 req/min → can monitor ~25 subreddits
- 1 search query at 60s interval = 1 req/min
- Reserve 5 req/min for ad-hoc queries
- Total budget: 30 req/min, plan accordingly
```

---

## Subreddit Analysis Framework

### Evaluating a Subreddit Before Posting

Before investing effort in any subreddit, run this assessment:

**Step 1 — Pull subreddit metadata**:
```bash
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/r/SUBREDDIT/about" | python3 -c "
import sys, json
d = json.load(sys.stdin)['data']
print(f'Subscribers: {d[\"subscribers\"]:,}')
print(f'Active now: {d[\"active_user_count\"]:,}')
print(f'Created: {d[\"created_utc\"]}')
print(f'Type: {d[\"submission_type\"]}')
print(f'Description: {d[\"public_description\"][:200]}')
"
```

**Step 2 — Measure actual engagement** (not just subscriber count):

```bash
# Get top 25 hot posts and examine their scores and comment counts
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/r/SUBREDDIT/hot?limit=25"
```

Calculate from the response:
```
Engagement Score = median(post_scores) * median(num_comments)
Activity Ratio = active_user_count / subscribers
Health Indicator = (posts_per_day > 5) AND (Activity Ratio > 0.001)
```

**Step 3 — Subreddit quality scorecard**:

| Factor | Good Sign | Bad Sign |
|--------|-----------|----------|
| Active/subscriber ratio | > 0.1% | < 0.01% |
| Median hot post score | > 50 | < 10 |
| Median comment count | > 10 | < 3 |
| Posts per day | 5-50 | < 1 or > 500 (noise) |
| Mod activity | Active modqueue, clear rules | No rules, spam in feed |
| Top post age | Within last 24h | Weeks old (dead subreddit) |
| Account age requirements | Reasonable (7 days) | None (spam-prone) or extreme (1 year) |

### Peak Engagement Hours by Subreddit Type

Optimal posting times vary by audience. All times in EST:

| Subreddit Type | Peak Hours | Peak Days | Reasoning |
|---------------|------------|-----------|-----------|
| Tech/Programming | 9-11 AM EST | Tue-Thu | Developers browse during morning coffee |
| Business/Startup | 7-9 AM EST | Mon-Wed | Professionals check before work |
| Gaming | 6-10 PM EST | Fri-Sun | After work/school |
| Science/Academic | 10 AM-12 PM EST | Mon-Wed | Researchers between tasks |
| Lifestyle/Hobby | 12-2 PM EST, 7-9 PM EST | Any | Lunch breaks and evenings |
| News/Politics | 7-9 AM EST | Mon-Fri | Morning news cycle |
| Finance/Crypto | 8-10 AM EST | Mon-Fri | Pre-market and market open |

To measure a specific subreddit's peak hours:
```bash
# Pull the last 100 posts and extract their timestamps
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/r/SUBREDDIT/new?limit=100"
# Parse created_utc for each post and bucket by hour-of-day
# Cross-reference with score to find high-score hours, not just high-volume hours
```

### Competitor Presence Analysis

**Step 1 — Search for competitor mentions**:
```bash
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/search?q=%22CompetitorName%22&sort=new&t=month&limit=100"
```

**Step 2 — Build a competitor activity profile**:
```
For each competitor, track:
- Which subreddits they are mentioned in (and by whom -- users vs the company)
- Frequency of mentions (per week)
- Sentiment of mentions (positive / neutral / negative)
- Whether they have official accounts engaging in threads
- Common complaints about them (your opportunity)
- Common praise for them (your benchmark)
```

**Step 3 — Competitor comparison matrix**:
| Metric | Your Brand | Competitor A | Competitor B |
|--------|-----------|-------------|-------------|
| Weekly mentions | | | |
| Positive sentiment % | | | |
| Subreddits present in | | | |
| Official account activity | | | |
| Top complaint theme | | | |
| Top praise theme | | | |

### Content Format Preferences by Subreddit Type

| Subreddit Type | Preferred Format | Avoid |
|---------------|-----------------|-------|
| Technical (r/programming, r/rust) | Long-form text, code blocks, benchmarks | Short posts, images without context |
| Q&A (r/AskReddit, r/askscience) | Concise questions, detailed answers | Link-only posts |
| Showcase (r/sideproject, r/webdev) | Screenshots, demos, before/after | Text-only without visuals |
| News (r/technology, r/science) | Link to source with summary comment | Self-post opinion pieces |
| Discussion (r/startups, r/cscareerquestions) | Personal experience, specific details | Generic advice, platitudes |
| Meme-friendly (r/ProgrammerHumor) | Images, short and punchy | Long text posts |

### Deep Subreddit Culture Analysis

Beyond reading the sidebar, you must understand how a subreddit actually behaves. Rules are the minimum; culture is what determines success.

**Step 1 — Analyze the top 25 hot posts for these signals:**
```
For each post, record:
- title_length: number of words
- body_length: number of words (0 if link post)
- is_question: does title end with "?"
- has_code_block: does body contain ``` or 4-space indent
- has_external_link: does body contain URLs
- flair_used: which flair, or none
- top_comment_style: first 3 top-level comments — length, tone, use of sources
```

**Step 2 — Build a subreddit profile:**
```
{
  "subreddit": "r/example",
  "median_title_words": 12,
  "median_body_words": 150,
  "question_post_ratio": 0.4,
  "code_block_frequency": 0.6,
  "link_post_ratio": 0.2,
  "dominant_flair": "Discussion",
  "comment_style": "detailed_technical",
  "avg_comment_length_words": 45,
  "humor_tolerance": "low",
  "self_promo_tolerance": "very_low",
  "newcomer_friendliness": "moderate"
}
```

**Step 3 — Adapt your behavior to the profile:**

| Profile Trait | Adaptation |
|--------------|------------|
| High `question_post_ratio` | Frame your posts as questions, even if sharing info |
| High `code_block_frequency` | Always include code examples in comments |
| Low `humor_tolerance` | No jokes, no puns, pure substance |
| Very low `self_promo_tolerance` | Never mention your own projects for at least 2 weeks of pure engagement |
| High `avg_comment_length_words` (>50) | Write detailed comments; one-liners will be ignored or downvoted |
| Low `avg_comment_length_words` (<20) | Keep it brief; walls of text will not be read |

### Cross-Subreddit Content Adaptation Strategy

When engaging with the same topic across multiple subreddits, each community requires a different approach. Never copy-paste.

**Adaptation matrix example — posting about "a new Rust CLI tool":**

| Aspect | r/rust | r/commandline | r/programming | r/sideproject |
|--------|--------|---------------|---------------|---------------|
| Title style | Technical: "crate_name: zero-copy CLI parser for X" | Practical: "crate_name -- does X in Y ms" | Neutral: "crate_name: an open-source CLI for X (Rust)" | Personal: "I built a CLI tool for X -- here's the story" |
| Body focus | Architecture, unsafe usage, benchmark vs alternatives | Install command, usage examples, screenshot | Link to repo, brief description, benchmark table | Motivation, challenges, what you learned |
| Expected length | 200-400 words + code | 100-200 words + demo gif | Link post with 2-3 sentence summary comment | 400-600 words narrative |
| Flair | "Tools & Libraries" | None typically | None | "Built This" |
| What to avoid | Marketing language, hype | Long explanations without examples | "Please star my repo" | Purely technical details without story |
| Comment engagement style | Deep technical discussion, benchmark methodology | "How does it handle edge case X?" | Brief, factual responses | Conversational, share the journey |

**Timing stagger for multi-subreddit posts:**
- Post first in the most niche subreddit (e.g., r/rust)
- Wait 4-8 hours, observe reception
- If positive (score > 10, good comments), adapt and post to the next subreddit
- Wait another 4-8 hours between each subsequent post
- Never post to more than 3 subreddits for the same content within 48 hours

---

## Growth & Reputation Building

### Karma Building Strategies (Comment-First Approach)

New accounts or accounts entering a new subreddit should follow the comment-first approach:

**Week 1-2: Listen and respond**
```
1. Sort by "new" in your target subreddits
2. Find questions you can genuinely answer
3. Write substantive, helpful comments (3+ sentences with specifics)
4. Respond to 3-5 threads per day
5. Do NOT mention your product, company, or project at all
```

**Week 3-4: Establish presence**
```
1. Start sharing relevant resources (not yours) that help the community
2. Engage in discussions about trends and opinions in your domain
3. Build recognition by being consistently helpful
4. Your username should start becoming familiar to regulars
```

**Week 5+: Contribute original content**
```
1. Share a technical write-up, tutorial, or analysis (unrelated to your product)
2. If well-received, you have earned the trust to occasionally mention your work
3. Always frame self-promotional content as "I built X" (transparent) not "Check out X" (spammy)
4. Maintain the 10:1 ratio — 10 helpful contributions for every 1 self-promotional post
```

Karma accumulation benchmarks:
| Milestone | Unlocks |
|-----------|---------|
| 10 comment karma | Bypass most anti-spam filters |
| 50 comment karma | Reduced posting cooldowns |
| 100+ comment karma in a subreddit | Trusted contributor status in some subreddits |
| 1000+ total karma | Access to r/lounge and some restricted subreddits |

### Building Authority in Niche Subreddits

Authority is built through consistency and expertise, not volume:

1. **Pick 3-5 subreddits maximum** -- spreading across 20 subreddits builds no authority anywhere
2. **Develop a recognizable voice** -- consistent formatting, depth of answers, specific expertise area
3. **Answer the hard questions** -- skip the easy ones that 10 people will answer; tackle the ones that require real expertise
4. **Follow up on your own answers** -- if someone asks a follow-up, respond promptly
5. **Cite sources and show work** -- "I benchmarked this myself, here are the numbers" is worth 100x "I think X is faster"
6. **Accept corrections gracefully** -- being wrong publicly and handling it well builds more trust than never being wrong

### Cross-Posting Etiquette and Strategy

Cross-posting (sharing a post from one subreddit to another) has specific norms:

**Do:**
- Use Reddit's built-in cross-post feature (preserves attribution)
- Cross-post to subreddits where the content genuinely fits
- Add a comment explaining why it is relevant to the new subreddit
- Wait at least a few hours between cross-posts (avoid appearing spammy)

**Don't:**
- Cross-post to more than 2-3 subreddits
- Cross-post to subreddits that explicitly ban it (check rules)
- Copy-paste the same text as a new post instead of cross-posting (treated as spam)
- Cross-post your own content excessively

**Strategic cross-posting pattern**:
```
1. Post original content in the most specific/niche subreddit first
2. If it gains traction (>20 upvotes, positive comments), cross-post to a broader subreddit
3. Customize the title for the new audience
4. Engage in comments on BOTH subreddits
```

### Handling Negative Feedback and Criticism

Negative feedback on Reddit is public and permanent. Handle it strategically:

**Response framework**:
```
1. PAUSE — Do not respond within the first 15 minutes. Emotional responses backfire.
2. ASSESS — Is the criticism valid, partially valid, or trolling?
3. RESPOND (or don't):
   - Valid criticism: Acknowledge, thank them, explain what you will do about it
   - Partially valid: Acknowledge the valid part, clarify the rest with facts
   - Trolling/bad faith: Do NOT respond. Silence is the best response.
4. FOLLOW UP — If you promised to fix something, come back and confirm when it is done
```

**Response templates by situation**:

| Situation | Response Pattern |
|-----------|-----------------|
| Bug report | "Thanks for reporting this. Can you share [details]? I've opened [issue link] to track it." |
| Feature complaint | "That's fair feedback. Here's why we made that choice: [reason]. We're considering [alternative]." |
| Unfair comparison | "Good question. Here's a direct comparison: [facts]. [Competitor] is great at X, we focus on Y." |
| Personal attack | Do not respond. Report if it violates rules. |
| "This is trash" | "Sorry it didn't work for you. What specifically went wrong? Happy to help." |

---

## Analytics & Reporting

### Post Performance Metrics

Key metrics to track for every post:

| Metric | Where to Find | What It Means |
|--------|--------------|---------------|
| Score | `data.score` | Net upvotes (upvotes minus downvotes) |
| Upvote ratio | `data.upvote_ratio` | 0.0-1.0, percentage of votes that are upvotes |
| Number of comments | `data.num_comments` | Total comments including replies |
| Awards | `data.all_awardings` | List of awards received |
| Cross-posts | `data.num_crossposts` | How many times others cross-posted it |

**Fetch post performance**:
```bash
curl -s -H "Authorization: Bearer $ACCESS_TOKEN" \
  -A "LibreFang Reddit Hand/1.0" \
  "https://oauth.reddit.com/by_id/t3_POST_ID"
```

**Quality indicators**:
```
High engagement:    upvote_ratio > 0.85 AND num_comments > 20
Controversial:      upvote_ratio 0.40-0.60 (heavily split votes)
Viral potential:    score > 100 within first 2 hours
Dead on arrival:    score < 5 after 4 hours
Comment quality:    avg comment length > 100 chars (real discussion vs memes)
```

### Engagement Trend Tracking

Track performance over time by recording metrics at regular intervals:

```
For each post, capture at:
- T+1 hour:   score, num_comments, upvote_ratio
- T+4 hours:  score, num_comments, upvote_ratio
- T+24 hours: score, num_comments, upvote_ratio (final snapshot)

For account-level tracking:
- Weekly comment karma change
- Weekly post karma change
- Number of posts/comments per subreddit
- Average score per post by subreddit
```

**Growth trajectory assessment**:
| Period | Healthy Growth | Stagnant | Declining |
|--------|---------------|----------|-----------|
| Weekly karma change | > +50 | -10 to +10 | < -10 |
| Avg post score trend | Increasing | Flat | Decreasing |
| Comment reply rate | > 30% of comments get replies | 10-30% | < 10% |
| New subreddit penetration | 1-2 new per month | 0 | Banned from any |

### ROI Measurement for Business-Related Reddit Activity

**Trackable outcomes**:
| Category | Metric | How to Track |
|----------|--------|-------------|
| Direct | Referral traffic | UTM parameters in shared links |
| Direct | Sign-ups/downloads | Reddit referral attribution |
| Direct | Support tickets deflected | Track answers that resolve issues |
| Direct | GitHub stars/forks | Append `?ref=reddit` to links |
| Indirect | Brand mention volume | Weekly search query tracking |
| Indirect | Sentiment ratio trend | Positive / total mentions over time |
| Indirect | Share of voice vs competitors | Compare mention counts monthly |

**ROI formula**: `(Value of outcomes - Total cost) / Total cost`
where cost = (hours/week * hourly rate) + content creation time + tool costs.

### Weekly Reddit Activity Report Template

```
## Reddit Activity Report — Week of [Date]
- Karma: [post] / [comment] (change: +/- [N]) | Removal rate: [N]%
- Posts: [N] (avg score: [N]) | Comments: [N] (avg score: [N])
- Notable: [Thread title] — [subreddit] — [link]
- Plan: [Target subreddits] | [Content planned]
```
