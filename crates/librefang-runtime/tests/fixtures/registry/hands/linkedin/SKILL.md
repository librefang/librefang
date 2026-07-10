---
name: linkedin-hand-skill
version: "1.0.0"
author: LibreFang
description: "Expert knowledge for AI LinkedIn management -- API reference, content strategy, networking playbook, and professional engagement best practices"
tags: [linkedin, social-media, professional, networking, content]
runtime: prompt_only
---

# LinkedIn Management Expert Knowledge

## LinkedIn API Reference

### Authentication
LinkedIn API uses OAuth 2.0 with bearer tokens.

**Bearer Token**:
```
Authorization: Bearer $LINKEDIN_ACCESS_TOKEN
```

### Core Endpoints

**Get authenticated user info**:
```bash
curl -s -H "Authorization: Bearer $LINKEDIN_ACCESS_TOKEN" \
  -H "LinkedIn-Version: 202405" \
  "https://api.linkedin.com/rest/userinfo"
```

**Create a text post (Posts API)**:
```bash
curl -s -X POST "https://api.linkedin.com/rest/posts" \
  -H "Authorization: Bearer $LINKEDIN_ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -H "LinkedIn-Version: 202405" \
  -d '{
    "author": "urn:li:person:MEMBER_ID",
    "lifecycleState": "PUBLISHED",
    "commentary": "Your post content here",
    "visibility": "PUBLIC",
    "distribution": {
      "feedDistribution": "MAIN_FEED"
    }
  }'
```

**Comment on a post**:
```bash
curl -s -X POST "https://api.linkedin.com/rest/socialActions/URN/comments" \
  -H "Authorization: Bearer $LINKEDIN_ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -H "LinkedIn-Version: 202405" \
  -d '{
    "actor": "urn:li:person:MEMBER_ID",
    "message": {"text": "Your comment here"}
  }'
```

**Like a post**:
```bash
curl -s -X POST "https://api.linkedin.com/rest/socialActions/URN/likes" \
  -H "Authorization: Bearer $LINKEDIN_ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -H "LinkedIn-Version: 202405" \
  -d '{
    "actor": "urn:li:person:MEMBER_ID"
  }'
```

### Rate Limits
| Endpoint | Limit | Window |
|----------|-------|--------|
| Posts | 25 posts | 24 hours |
| Comments | 10 comments | 1 minute |
| Likes | 20 likes | 1 minute |
| API calls (general) | 100 requests | 1 day |

---

## LinkedIn Content Strategy

### The LinkedIn Algorithm (2024-2025)

Key factors that affect reach:
1. **Dwell time**: How long people spend reading your post. LinkedIn tracks both "read dwell" (time spent on post text) and "click dwell" (time spent after clicking "see more"). Longer posts that hold attention get amplified. Ideal: 800-1300 characters that reward reading to the end.
2. **Early engagement**: Comments in the first 60-90 minutes are weighted heavily. The algorithm decides distribution tiers within 2 hours of posting.
3. **Meaningful comments**: Long comments (3+ sentences) signal quality far more than likes. One thoughtful comment is worth ~10 likes in the algorithm. Reply-to-reply threads (nested comments) further boost the post.
4. **No external links**: Posts with links get 40-50% less reach. The algorithm deprioritizes anything that drives users off-platform.
5. **Personal stories**: Narrative content outperforms promotional content. The algorithm favors "knowledge and advice" posts from individuals over brand content.

**Engagement signal weighting** (approximate relative impact on distribution):
| Signal | Relative Weight | Why |
|--------|----------------|-----|
| Comment (3+ sentences) | 10x | Strongest indicator of quality content |
| Repost with commentary | 8x | Shows content worth sharing and adding to |
| Save/bookmark | 6x | High-intent signal — user wants to revisit |
| Reply in comment thread | 5x | Sustained conversation signals value |
| Share (plain repost) | 4x | Distribution signal but lower intent |
| Reaction (any emoji) | 1x | Baseline engagement, lowest weight |
| Click "see more" | 0.5x | Curiosity signal, but no follow-through guarantee |

**Algorithm penalty signals**:
- Editing a post within 10 minutes of publishing can reset distribution
- Deleting and reposting gets flagged and suppressed
- Posting more than once per 18 hours splits your audience
- Engagement pods (coordinated likes/comments) are detected and penalized
- Hashtag stuffing (>5) triggers spam signals

### Content Pillars

Define 3-4 content pillars:
```
Example for a tech leader:
  Pillar 1: Engineering Leadership (40%)
  Pillar 2: Industry Trends & Analysis (30%)
  Pillar 3: Career Growth & Mentoring (20%)
  Pillar 4: Personal Lessons (10%)
```

### Post Formats That Work

| Format | Avg Engagement | Best For |
|--------|---------------|----------|
| Personal story with lesson | High | Connection, authenticity |
| Contrarian take | High | Discussion, visibility |
| Step-by-step guide | Medium-High | Authority, saves |
| Data + insight | Medium | Credibility |
| Question/poll | Medium | Engagement |
| Industry news + analysis | Medium | Thought leadership |

### Optimal Posting Times (UTC-based)

| Day | Best Times | Why |
|-----|-----------|-----|
| Tuesday | 8-10 AM | Peak professional engagement |
| Wednesday | 8-10 AM | Mid-week content consumption |
| Thursday | 8-10 AM, 12 PM | Second-best engagement day |
| Monday | 8-10 AM | Start of work week |
| Friday | 8-9 AM only | Engagement drops after morning |
| Weekend | Avoid | 60-70% lower engagement |

---

## Post Writing Best Practices

### The Hook (First 2 Lines)

The first 2 lines appear before the "see more" fold. They must compel a click.

Hooks that work:
- **Bold statement**: "I fired my best employee last week. Here's why it was the right call."
- **Surprising data**: "Only 3% of engineering managers do this. It changes everything."
- **Confession**: "I made a $500K mistake in my first year as CTO."
- **Question**: "Why do 90% of digital transformations fail?"
- **Contrarian**: "Unpopular opinion: Stand-ups are a waste of time."

### Writing Rules

1. **One idea per post** -- don't try to cover everything
2. **Short paragraphs** -- 1-2 sentences max, lots of white space
3. **Use line breaks** -- make it scannable
4. **End with a question** -- drives comments which boost reach
5. **No links in the post body** -- put links in the first comment
6. **3-5 relevant hashtags** -- at the bottom of the post
7. **1000-1300 characters** -- sweet spot for engagement
8. **Be authentic** -- personal stories outperform corporate speak

### Comment Strategy

When commenting on others' posts:
- Add a new perspective or data point
- Share a relevant personal experience
- Ask a thoughtful follow-up question
- Keep comments 2-4 sentences (meaningful but concise)
- Avoid generic comments ("Great post!", "Thanks for sharing!")

---

## Networking Best Practices

### Connection Requests
- Always add a personal note (not the default message)
- Reference something specific (their content, mutual connection, shared interest)
- Keep it under 300 characters
- Don't pitch in the connection request

### Relationship Building
- Consistently engage with connections' content before asking for anything
- Share others' content with genuine commentary
- Celebrate connections' achievements publicly
- Offer help or resources without expecting anything in return

---

## Safety & Compliance

### Content Guidelines
NEVER post:
- Confidential business information
- Discriminatory or offensive content
- False credentials or experience claims
- Defamatory statements about competitors or individuals
- Content that violates LinkedIn's Professional Community Policies
- Misleading data or fabricated statistics

### Content Moderation Rules

Before posting any content, classify it:

**Auto-REJECT** (never post):
- Content containing hate speech, discrimination, or harassment
- Unverified claims about competitors or individuals
- Confidential or proprietary business information
- Content that could be interpreted as financial or legal advice
- Anything with profanity or inappropriate language
- Political or religious debate content

**Flag for REVIEW** (queue for human approval):
- Controversial industry opinions or contrarian takes
- Content mentioning specific companies or individuals by name
- Posts discussing salary, compensation, or workplace issues
- Content referencing current news events
- Posts with strong emotional tone or personal vulnerability

**Safe to POST** (can auto-publish if approval_mode is off):
- Educational how-to content and professional tips
- Industry trend analysis with cited sources
- Career development advice and frameworks
- Engagement posts (professional questions, polls)
- Celebration of team or industry achievements

### Professional Standards
- Maintain professional tone even in casual posts
- Fact-check all claims and statistics
- Credit sources and tag collaborators
- Disclose affiliations when discussing products or services
- Respect intellectual property and copyright

### Crisis Management for Negative Engagement

When a post receives significant negative attention (hostile comments, public disagreements, misinterpretation):

**Severity levels and response**:

| Level | Indicators | Action |
|-------|-----------|--------|
| **Low** | 1-2 disagreeing comments, professional tone | Respond thoughtfully; treat as healthy discussion |
| **Medium** | Multiple negative comments, some personal attacks, post being quote-shared critically | Pause auto-engagement; draft a measured clarification comment; queue for user review |
| **High** | Viral negative attention, accusations of misinformation, brand/employer reputation risk | Alert user immediately via event_publish "linkedin_crisis_alert"; do NOT auto-respond; prepare a response draft for human approval |

**Response playbook**:
1. **Never delete a post** that has active engagement -- it signals guilt and people screenshot first
2. **Never argue in comment threads** -- one measured response per critic, then disengage
3. **Acknowledge valid criticism** gracefully: "That's a fair point -- I should have been clearer about [X]. Here's what I meant: ..."
4. **For factual errors** in your post: Add a correction comment pinned at the top: "Update: [correction]. Thanks to @Name for pointing this out."
5. **For personal attacks**: Do not respond. Hide the comment (LinkedIn allows this) and move on. If persistent, report to LinkedIn.
6. **For misinterpretation at scale**: Write a follow-up post (not an edit) that clarifies the original point without being defensive

**After a crisis**: Log the incident in memory, note what triggered it, and update content moderation rules to prevent recurrence.

---

## Advanced API Patterns

### Image Post Creation (Media Upload Flow)

Posting an image requires a 3-step flow: register upload, upload binary, then create post.

**Step 1 -- Register the upload**:
```bash
curl -s -X POST "https://api.linkedin.com/rest/images?action=initializeUpload" \
  -H "Authorization: Bearer $LINKEDIN_ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -H "LinkedIn-Version: 202405" \
  -d '{
    "initializeUploadRequest": {
      "owner": "urn:li:person:MEMBER_ID"
    }
  }'
```
Response contains `uploadUrl` and `image` URN (e.g., `urn:li:image:C4E...`).

**Step 2 -- Upload the binary**:
```bash
curl -s -X PUT "$UPLOAD_URL" \
  -H "Authorization: Bearer $LINKEDIN_ACCESS_TOKEN" \
  -H "Content-Type: image/png" \
  --data-binary "@/path/to/image.png"
```

**Step 3 -- Create post with image**:
```bash
curl -s -X POST "https://api.linkedin.com/rest/posts" \
  -H "Authorization: Bearer $LINKEDIN_ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -H "LinkedIn-Version: 202405" \
  -d '{
    "author": "urn:li:person:MEMBER_ID",
    "lifecycleState": "PUBLISHED",
    "commentary": "Check out our Q3 results!",
    "visibility": "PUBLIC",
    "distribution": {"feedDistribution": "MAIN_FEED"},
    "content": {
      "media": {
        "id": "urn:li:image:IMAGE_URN",
        "title": "Q3 Performance Summary"
      }
    }
  }'
```

### Document Post Creation (PDF/Carousel)

LinkedIn "document posts" (carousels) follow the same register-upload-post pattern but use the documents API.

**Register document upload**:
```bash
curl -s -X POST "https://api.linkedin.com/rest/documents?action=initializeUpload" \
  -H "Authorization: Bearer $LINKEDIN_ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -H "LinkedIn-Version: 202405" \
  -d '{
    "initializeUploadRequest": {
      "owner": "urn:li:person:MEMBER_ID"
    }
  }'
```

**Upload the PDF and create post** (same pattern as image -- PUT binary, then POST with `content.media.id` set to the document URN).

### Article Publishing via API

**Create an article post** (link article hosted externally):
```bash
curl -s -X POST "https://api.linkedin.com/rest/posts" \
  -H "Authorization: Bearer $LINKEDIN_ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -H "LinkedIn-Version: 202405" \
  -d '{
    "author": "urn:li:person:MEMBER_ID",
    "lifecycleState": "PUBLISHED",
    "commentary": "I wrote about why most engineering teams get incident response wrong.\n\nKey insight: the 5-minute rule changes everything.",
    "visibility": "PUBLIC",
    "distribution": {"feedDistribution": "MAIN_FEED"},
    "content": {
      "article": {
        "source": "https://yourblog.com/incident-response",
        "title": "The 5-Minute Rule for Incident Response",
        "description": "A practical framework for engineering teams"
      }
    }
  }'
```

> **Note**: Article-link posts get reduced reach vs native text posts. Prefer putting links in the first comment.

### Analytics Endpoints

**Get post statistics (organic)**:
```bash
curl -s -H "Authorization: Bearer $LINKEDIN_ACCESS_TOKEN" \
  -H "LinkedIn-Version: 202405" \
  "https://api.linkedin.com/rest/organizationalEntityShareStatistics?q=organizationalEntity&organizationalEntity=urn:li:organization:ORG_ID&timeIntervals.timeGranularityType=DAY&timeIntervals.timeRange.start=1704067200000&timeIntervals.timeRange.end=1706745600000"
```

**Get share statistics for a specific post**:
```bash
curl -s -H "Authorization: Bearer $LINKEDIN_ACCESS_TOKEN" \
  -H "LinkedIn-Version: 202405" \
  "https://api.linkedin.com/rest/organizationalEntityShareStatistics?q=organizationalEntity&organizationalEntity=urn:li:organization:ORG_ID&shares=urn:li:share:SHARE_ID"
```

Response fields:
| Field | Description |
|-------|-------------|
| `totalShareStatistics.impressionCount` | Total times the post appeared in feeds |
| `totalShareStatistics.uniqueImpressionsCount` | Unique viewers |
| `totalShareStatistics.clickCount` | Total clicks (content + read more) |
| `totalShareStatistics.likeCount` | Total likes/reactions |
| `totalShareStatistics.commentCount` | Total comments |
| `totalShareStatistics.shareCount` | Total reposts |
| `totalShareStatistics.engagement` | Engagement rate (decimal) |

**Get follower statistics** (organization pages):
```bash
curl -s -H "Authorization: Bearer $LINKEDIN_ACCESS_TOKEN" \
  -H "LinkedIn-Version: 202405" \
  "https://api.linkedin.com/rest/organizationalEntityFollowerStatistics?q=organizationalEntity&organizationalEntity=urn:li:organization:ORG_ID"
```

### Webhook / Notification Patterns

LinkedIn does not offer real-time webhooks for most events. Use polling instead:

```
Polling strategy:
  - Post engagement: Poll every 15 minutes for first 4 hours after posting
  - Mentions/comments: Poll every 5 minutes during engagement_hours
  - Follower counts: Poll once per day
  - Analytics: Poll once per day (data lags 24-48 hours)
```

### Error Handling and Rate Limit Retry

```
Rate limit response headers:
  X-RateLimit-Limit: 100
  X-RateLimit-Remaining: 0
  X-RateLimit-Reset: 1706745600

HTTP 429 response:
  {"status": 429, "message": "Resource level throttle limit..."}
```

**Retry strategy**:
```
1. On HTTP 429: Read X-RateLimit-Reset header
2. Calculate wait_seconds = reset_timestamp - current_timestamp
3. Sleep for wait_seconds + 1 (buffer)
4. Retry the request (max 3 retries)
5. On 3 consecutive 429s: back off for 15 minutes

On HTTP 5xx (server error):
1. Retry with exponential backoff: 1s, 2s, 4s
2. Max 3 retries
3. Log failure and queue for later retry

On HTTP 401 (expired token):
1. Trigger OAuth 2.0 refresh flow
2. Update stored token
3. Retry original request once
```

---

## Content Calendar Template

### Monthly Content Planning Framework

Organize content around weekly themes that rotate through your content pillars.

```
MONTH: [Month Year]
THEME ROTATION:
  Week 1: [Pillar 1 -- e.g., Engineering Leadership]
  Week 2: [Pillar 2 -- e.g., Industry Trends]
  Week 3: [Pillar 3 -- e.g., Career Growth]
  Week 4: [Pillar 1 deep dive OR seasonal/timely topic]
```

### Weekly Content Schedule

```
WEEK OF [DATE] — Theme: [Weekly Theme]

Monday:
  - 8:30 AM: [Personal story] tied to weekly theme
    Format: Hook + narrative + lesson + question
    Goal: High engagement to start the week

Tuesday:
  - 9:00 AM: [Step-by-step guide] or [How-to]
    Format: Numbered list with tactical advice
    Goal: Saves and shares (authority building)

Wednesday:
  - 8:30 AM: [Data/insight post] with original analysis
    Format: Stat + context + your take + question
    Goal: Credibility and thought leadership

Thursday:
  - 9:00 AM: [Contrarian take] or [Industry opinion]
    Format: Bold statement + reasoning + invitation to debate
    Goal: Comments and discussion (algorithm boost)

Friday:
  - 8:00 AM: [Engagement post] — poll, question, or lightweight personal content
    Format: Short, conversational, easy to respond to
    Goal: Community building before weekend
```

### Content Mix Ratios

| Category | % of Posts | Examples |
|----------|-----------|----------|
| Educational / Value | 40% | How-tos, frameworks, lessons learned |
| Personal / Storytelling | 25% | Career stories, failures, reflections |
| Engagement / Discussion | 20% | Questions, polls, contrarian takes |
| Promotional / Company | 10% | Product launches, hiring, milestones |
| Curated / Commentary | 5% | Industry news with your analysis |

**Rule**: Never let promotional content exceed 15%. LinkedIn penalizes overtly sales-y accounts.

### Engagement Windows and Response Strategy

```
Post published at 8:30 AM:
  Minutes 0-15:  Reply to EVERY comment immediately (signals activity to algorithm)
  Minutes 15-60: Reply within 5 minutes of each new comment
  Hours 1-4:     Reply within 30 minutes
  Hours 4-24:    Reply within 2 hours (during business hours)
  Day 2+:        Reply within 24 hours

First-comment strategy:
  - Post your own comment within 2 minutes of publishing
  - Use it for: link to resource, additional context, question to spark discussion
  - This comment acts as engagement seed
```

---

## Analytics & Optimization

### Key Metrics to Track

| Metric | Formula | Good Benchmark | Great Benchmark |
|--------|---------|----------------|-----------------|
| Engagement rate | (reactions + comments + reposts) / impressions | > 2% | > 5% |
| Comment rate | comments / impressions | > 0.3% | > 1% |
| Follower growth rate | net new followers / total followers per week | > 0.5% | > 2% |
| Profile views | weekly profile views trend | Consistent growth | 2x after viral post |
| SSI (Social Selling Index) | LinkedIn's built-in score (0-100) | > 50 | > 70 |
| Content saves | saves / impressions | > 0.5% | > 2% |
| Click-through rate | clicks / impressions | > 1% | > 3% |

### Engagement Rate Calculation

```
engagement_rate = (reactions + comments + reposts) / impressions * 100

Example:
  120 reactions + 35 comments + 8 reposts = 163 engagements
  163 / 5,200 impressions = 3.13% engagement rate

Per-post tracking:
  | Post Date | Topic | Format | Impressions | Eng Rate | Comments |
  |-----------|-------|--------|-------------|----------|----------|
  | Mon 03/03 | Leadership | Story | 5,200 | 3.13% | 35 |
  | Tue 03/04 | AI Tools | How-to | 3,800 | 4.21% | 22 |
  | Wed 03/05 | Hiring | Data | 2,100 | 2.85% | 12 |
```

### A/B Testing Strategies

Test one variable at a time across pairs of similar posts:

| Variable | Option A | Option B | Track |
|----------|----------|----------|-------|
| Hook style | Question hook | Bold statement hook | Click-through rate |
| Post length | Short (< 800 chars) | Long (1200+ chars) | Dwell time, engagement |
| Posting time | 8:00 AM | 9:30 AM | Impressions after 4 hours |
| CTA type | Question CTA | "Agree? Repost" CTA | Comment rate vs repost rate |
| Hashtag count | 3 hashtags | 0 hashtags | Reach beyond network |
| Format | Plain text | Text + image | Engagement rate |

**How to run a test**:
1. Pick one variable to test (e.g., posting time)
2. Keep everything else constant (same pillar, similar format, similar length)
3. Run for 2 weeks (minimum 4 posts per variant)
4. Compare average metrics -- ignore outliers
5. Adopt the winner and move to next variable

**A/B test tracking template** (store in `linkedin_ab_tests.json`):
```json
{
  "test_id": "test-hook-style-001",
  "variable": "hook_style",
  "hypothesis": "Bold statement hooks generate higher click-through than question hooks",
  "start_date": "2025-03-10",
  "end_date": "2025-03-24",
  "status": "running",
  "variant_a": {
    "description": "Question hook",
    "post_ids": ["q-20250310-001", "q-20250312-001", "q-20250314-001"],
    "avg_impressions": 3200,
    "avg_engagement_rate": 2.8,
    "avg_comment_rate": 0.4
  },
  "variant_b": {
    "description": "Bold statement hook",
    "post_ids": ["q-20250311-001", "q-20250313-001", "q-20250315-001"],
    "avg_impressions": 4100,
    "avg_engagement_rate": 3.5,
    "avg_comment_rate": 0.6
  },
  "conclusion": null
}
```

**Statistical rigor**: With LinkedIn's natural variance, require at least 4 posts per variant and a >20% difference in the primary metric before declaring a winner. If the difference is <20%, the test is inconclusive -- run for another week or accept that the variable does not materially affect performance.

### Identifying Top-Performing Content Patterns

After 30+ posts, analyze your data to find patterns:

```
Sort all posts by engagement rate (descending):
  1. Look at your top 5 posts — what do they share?
     - Same content pillar?
     - Same format (story, how-to, contrarian)?
     - Same hook style?
     - Similar length range?
     - Same posting day/time?
  2. Look at your bottom 5 posts — what went wrong?
     - External links in body?
     - Promotional tone?
     - Published on Friday/weekend?
     - Weak hook?
  3. Create your "hit formula":
     Best combo: [Pillar] + [Format] + [Hook style] + [Day/Time]
     Example: "Engineering Leadership + Personal Story + Confession Hook + Tuesday 8:30 AM"
```

---

## Audience Growth Strategies

### Comment-First Strategy

The fastest way to grow on LinkedIn is strategic commenting on high-visibility posts.

**How it works**:
1. Identify 15-20 active creators in your niche (10K+ followers)
2. Turn on notifications for their posts
3. Be among the first 5 comments on their new posts
4. Write substantive comments (3-5 sentences) that add genuine value

**Comment templates for growth**:
```
Adding a data point:
  "This resonates. At [Company/Role], we saw [specific metric] when we
  implemented [related approach]. The key difference was [insight].
  Curious if others have seen similar results?"

Respectful counterpoint:
  "Interesting perspective. I'd push back slightly on [point] — in my
  experience with [context], the opposite was true because [reason].
  That said, I think [original point] absolutely holds for [use case]."

Extending the idea:
  "Building on this — one thing I'd add is [new angle]. I wrote about
  this recently and the biggest takeaway was [specific insight].
  [Question that invites further discussion]?"
```

**Target**: 5-10 thoughtful comments per day during peak hours (8-10 AM).

### Collaborative Content Patterns

**Tagging strategy**:
- Tag 1-3 people who would genuinely find the content relevant
- Always explain WHY you're tagging them (not drive-by tags)
- Tag people you've already engaged with (they're more likely to respond)

```
Example post with strategic tags:
  "I've been thinking about how engineering teams handle on-call rotations.

  After talking to 20+ eng managers, here are the 3 models that actually work:

  1. Follow-the-sun (best for distributed teams)
  2. Volunteer-first rotation (best for small teams)
  3. Tiered escalation (best for complex systems)

  @Name1 — your team's approach to #2 was eye-opening.
  @Name2 — curious if your distributed team uses #1 or something else?

  What model does your team use? Reply with your team size."
```

**Co-creation patterns**:
- Interview a peer and post key insights (tag them, they reshare)
- "X people I learned from this year" posts (mass tagging, high reshare rate)
- Collaborative lists: "Drop your best [resource] in the comments, I'll compile and share"

### LinkedIn Newsletter Strategy

Newsletters convert profile visitors into subscribers with direct inbox delivery.

**Newsletter setup checklist**:
```
1. Name: Clear, specific, benefit-driven
   Good: "The Engineering Leader's Playbook"
   Bad: "My Thoughts on Things"

2. Cadence: Weekly or biweekly (consistency > frequency)

3. Format:
   - 800-1500 words (longer than posts, shorter than blog articles)
   - One core idea per issue
   - Actionable takeaways or frameworks
   - End with a question to drive comments

4. Promotion:
   - Announce each issue with a teaser post (don't just auto-share)
   - Reference newsletter content in regular posts
   - Cross-promote with other newsletter authors
```

**Newsletter content structure**:
```
Issue #[N]: [Compelling Title]

[Hook paragraph -- why this matters NOW]

[Section 1: The Problem / Context]
  - 2-3 paragraphs with specific examples

[Section 2: The Framework / Solution]
  - Numbered steps or clear model
  - Real-world application examples

[Section 3: How to Apply This]
  - Actionable next steps the reader can take today

[Closing: Question + CTA]
  "What's your experience with [topic]? Reply in the comments."
  "If you found this useful, share it with your team."
```

### LinkedIn Live and Events

**LinkedIn Live** broadcasts get 7x more reactions and 24x more comments than regular video posts.

**Live session framework**:
```
Pre-event (1 week before):
  - Create LinkedIn Event and post announcement
  - Send invites to relevant connections
  - Post 2-3 teaser posts building anticipation

During event:
  - Start 2 minutes early for tech check
  - Open with clear agenda (30 seconds)
  - Acknowledge live commenters by name
  - Keep sessions 20-40 minutes

Post-event:
  - Post key takeaways within 2 hours
  - Reply to all comments on the event post
  - Repurpose recording into 3-5 short clips for future posts
```

**Event types that work**:
| Type | Duration | Best For | Frequency |
|------|----------|----------|-----------|
| AMA (Ask Me Anything) | 30 min | Engagement, authority | Monthly |
| Industry deep dive | 20 min | Thought leadership | Biweekly |
| Interview / fireside chat | 40 min | Network growth | Monthly |
| Quick tip / hot take | 10 min | Visibility | Weekly |

---

## Worked Examples

### Example 1: Thought Leadership Campaign

**Scenario**: VP of Engineering building authority in "engineering culture" niche.

**Content pillars**:
```
Pillar 1: Engineering Management (40%)
Pillar 2: Scaling Teams (30%)
Pillar 3: Career Advice (20%)
Pillar 4: Personal Lessons (10%)
```

**Week 1 posting schedule with sample posts**:

**Monday 8:30 AM -- Personal Story (Pillar 1)**:
```
I promoted my worst interviewer to Head of Recruiting.

Sounds crazy. Here's what happened.

She kept rejecting candidates everyone else loved.
Her "pass rate" was 15%. Team average was 60%.

But after 12 months, something became clear:

Her hires had:
→ 94% retention rate (team avg: 71%)
→ 2.3x faster time to first meaningful contribution
→ Zero PIPs in their first year

She wasn't a bad interviewer.
She was the only one actually doing it right.

The lesson?
Measure what matters. Pass rates reward speed.
Retention rates reward judgment.

What's one metric your team optimizes for
that might be the wrong one?

#EngineeringLeadership #Hiring #TechManagement
```

**Tuesday 9:00 AM -- How-To Guide (Pillar 2)**:
```
How to run a team retrospective that people actually enjoy
(not the soul-crushing ones everyone dreads):

Step 1: Kill the "what went well / what didn't" format
→ Use "I wish... I wonder... I'm proud of..." instead

Step 2: Timebox ruthlessly
→ 45 minutes max. If it takes longer, your team is too big for one retro.

Step 3: One action item per person, max
→ A retro with 20 action items produces zero change.
→ One item per person = accountability.

Step 4: Start with appreciation
→ First 5 minutes: each person thanks someone else on the team.
→ This changes the entire energy of the room.

Step 5: Rotate the facilitator
→ The manager should NOT always run retros.
→ It changes what people feel safe saying.

I've used this format with teams of 5 to teams of 50.

What's your retro format? Drop it below --
I'm always looking for new approaches.

#Agile #EngineeringCulture #TeamManagement
```

**Wednesday 8:30 AM -- Data + Insight (Pillar 2)**:
```
We tracked every engineering team meeting for 6 months.

The data was uncomfortable.

→ Average engineer: 11.2 hours/week in meetings
→ Senior engineers: 16.4 hours/week
→ Time spent in meetings that could've been async: 62%

We cut 40% of recurring meetings.

Result after 3 months:
→ Sprint velocity: +23%
→ Engineer satisfaction: +31% (internal survey)
→ "Deep work" blocks per week: 2.1 → 4.7

The surprising part?
Nobody missed the deleted meetings.
Not one person asked to bring them back.

If you haven't audited your meeting load recently,
you're probably burning 30-40% of your team's capacity.

What % of your meetings could be an async update?

#Engineering #Productivity #Leadership
```

**Thursday 9:00 AM -- Contrarian Take (Pillar 3)**:
```
Unpopular opinion: "Culture fit" interviews should be illegal.

Here's why:

Culture fit = "do I want to get a beer with this person?"
That's not hiring. That's friend-making.

What actually matters:
→ Values alignment (do they care about the same outcomes?)
→ Working style compatibility (async vs sync, docs vs meetings)
→ Growth trajectory (will they push the team forward?)

None of those require "fitting in."

The best hire I ever made was someone who challenged
every assumption we had. They didn't "fit" our culture.

They made it better.

Replace "culture fit" with "culture add."

Agree or disagree? I'd love to hear your take.

#Hiring #Diversity #EngineeringCulture #Leadership
```

**Friday 8:00 AM -- Engagement Post (Pillar 4)**:
```
Fill in the blank:

"The best career advice I ever received was ___________."

I'll go first:

"Stop optimizing for your next promotion.
Start optimizing for your next learning curve."

Changed how I made every career decision since.

Your turn.

#CareerAdvice #ProfessionalGrowth
```

### Example 2: Company Page Management

**Scenario**: B2B SaaS company (Series B, 80 employees) managing their LinkedIn company page.

**Posting cadence**:
```
Company page: 4-5 posts per week
Employee advocacy: 2-3 employees reshare/post per week
Executive accounts: CEO + CTO post 2-3x/week each
```

**Weekly company page schedule**:
```
Monday:    Industry insight or thought leadership (educational)
Tuesday:   Product tip or customer use case (value-driven)
Wednesday: Team/culture spotlight (employer branding)
Thursday:  Data or trend analysis (authority)
Friday:    Milestone, hiring, or community post (engagement)
```

**Sample company page posts**:

**Tuesday -- Customer Use Case**:
```
"We used to spend 3 hours every Monday pulling reports manually."

That's what @CustomerName's ops team told us last quarter.

After switching to [Product] automated workflows:
→ Report generation: 3 hours → 12 minutes
→ Data accuracy: 89% → 99.7%
→ Team freed up: 12 hours/week for strategic work

The best part? They set it up in a single afternoon.

Read the full story: [link in first comment]

#DataAutomation #Operations #CustomerSuccess
```

**Wednesday -- Team Culture Spotlight**:
```
This is Sarah. She joined us as intern #3 two years ago.

Last week she deployed our new ML pipeline to production.
By herself. On a Tuesday. No drama.

What happened in between:
→ Mentored by 4 different senior engineers
→ Shipped 47 PRs in her first year
→ Gave her first conference talk at 23
→ Now leads a team of 3

We don't hire for credentials.
We hire for curiosity and grit.

Sarah had both.

We're hiring 5 more engineers just like her.
Link in the comments.

#Hiring #Engineering #StartupCulture #WomenInTech
```

**Employee advocacy tracking**:
```
| Employee | Role | Posts/Week | Avg Reach | Topics |
|----------|------|-----------|-----------|--------|
| CEO | Executive | 3 | 8,500 | Vision, industry, leadership |
| CTO | Executive | 2 | 5,200 | Technical, architecture, hiring |
| VP Eng | Leader | 2 | 3,100 | Engineering culture, management |
| DevRel | IC | 3 | 4,800 | Tutorials, product, community |
```

**Analytics tracking cadence**:
```
Daily:   Check post-level engagement (reactions, comments, shares)
Weekly:  Follower growth, top-performing post, engagement rate trend
Monthly: Content audit — which pillars/formats performed best
         Adjust next month's content mix based on data
Quarterly: Competitor benchmarking, SSI review, strategy refresh
```

### Example 3: Job Seeker Profile Optimization Campaign

**Scenario**: Senior developer transitioning to engineering management role.

**4-week content plan**:
```
Week 1: Establish expertise
  - Post about a technical decision you led and its business impact
  - Share a "lessons from my first year managing" story
  - Comment on 10 engineering leadership posts

Week 2: Demonstrate thought leadership
  - Publish a how-to post: "How I transitioned from IC to manager"
  - Share data or a framework you've developed
  - Start engaging with hiring managers' content in target companies

Week 3: Build social proof
  - Post about a mentoring success story (tag the mentee with permission)
  - Share a "things I wish I knew" post targeting new managers
  - Request 3-5 recommendations from colleagues and reports

Week 4: Signal availability
  - Post about what you're looking for (without desperation)
  - Engage heavily in target company employees' content
  - Send personalized connection requests to hiring managers
```

**Sample "open to opportunities" post**:
```
After 8 years of writing code and 2 years of leading teams,
I'm looking for my next engineering management challenge.

What I bring to the table:
→ Scaled a team from 4 to 22 engineers
→ Reduced deployment failures by 73% through better process
→ Mentored 6 engineers into senior roles
→ Built hiring pipelines that maintained 85%+ offer acceptance

What I'm looking for:
→ Series A-C company building something meaningful
→ Team of 8-20 engineers who care about craft
→ Leadership that values engineering culture, not just velocity

If your team is growing and you value
managers who still understand the code --
I'd love to chat.

DMs are open. Or drop a comment and I'll reach out.

#OpenToWork #EngineeringManager #Hiring #Leadership
```
