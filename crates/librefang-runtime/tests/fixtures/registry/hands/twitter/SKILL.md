---
name: twitter-hand-skill
version: "1.0.0"
description: "Expert knowledge for AI Twitter/X management — API v2 reference, content strategy, engagement playbook, safety, and performance tracking"
runtime: prompt_only
---

# Twitter/X Management Expert Knowledge

## Twitter API v2 Reference

### Authentication
Twitter API v2 uses OAuth 2.0 Bearer Token for app-level access and OAuth 1.0a for user-level actions.

**Bearer Token** (read-only access + tweet creation):
```
Authorization: Bearer $TWITTER_BEARER_TOKEN
```

**Environment variable**: `TWITTER_BEARER_TOKEN`

**Authentication modes**:
- **Bearer Token only** (default): Sufficient for posting tweets, reading timelines, and searching. All core functionality works with just the Bearer Token.
- **Bearer Token + OAuth 1.0a** (optional): Required for user-context operations such as liking tweets, retweeting, following/unfollowing, and accessing DMs. Set `TWITTER_API_KEY`, `TWITTER_API_SECRET`, `TWITTER_ACCESS_TOKEN`, and `TWITTER_ACCESS_TOKEN_SECRET` to enable these features.

> **Note**: The Like endpoint (`POST /2/users/:id/likes`) and Retweet endpoint (`POST /2/users/:id/retweets`) require OAuth 1.0a User Context authentication. If only Bearer Token is configured, these operations will be skipped with a warning.

### Core Endpoints

**Get authenticated user info**:
```bash
curl -s -H "Authorization: Bearer $TWITTER_BEARER_TOKEN" \
  "https://api.twitter.com/2/users/me"
```
Response: `{"data": {"id": "123", "name": "User", "username": "user"}}`

**Post a tweet**:
```bash
curl -s -X POST "https://api.twitter.com/2/tweets" \
  -H "Authorization: Bearer $TWITTER_BEARER_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"text": "Hello world!"}'
```
Response: `{"data": {"id": "tweet_id", "text": "Hello world!"}}`

**Post a reply**:
```bash
curl -s -X POST "https://api.twitter.com/2/tweets" \
  -H "Authorization: Bearer $TWITTER_BEARER_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"text": "Great point!", "reply": {"in_reply_to_tweet_id": "PARENT_TWEET_ID"}}'
```

**Post a thread** (chain of replies to yourself):
1. Post first tweet → get `tweet_id`
2. Post second tweet with `reply.in_reply_to_tweet_id` = first tweet_id
3. Repeat for each tweet in thread

**Delete a tweet**:
```bash
curl -s -X DELETE "https://api.twitter.com/2/tweets/TWEET_ID" \
  -H "Authorization: Bearer $TWITTER_BEARER_TOKEN"
```

**Like a tweet**:
```bash
curl -s -X POST "https://api.twitter.com/2/users/USER_ID/likes" \
  -H "Authorization: Bearer $TWITTER_BEARER_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"tweet_id": "TARGET_TWEET_ID"}'
```

**Get mentions**:
```bash
curl -s -H "Authorization: Bearer $TWITTER_BEARER_TOKEN" \
  "https://api.twitter.com/2/users/USER_ID/mentions?max_results=10&tweet.fields=public_metrics,created_at,author_id"
```

**Search recent tweets**:
```bash
curl -s -H "Authorization: Bearer $TWITTER_BEARER_TOKEN" \
  "https://api.twitter.com/2/tweets/search/recent?query=QUERY&max_results=10&tweet.fields=public_metrics"
```

**Get tweet metrics**:
```bash
curl -s -H "Authorization: Bearer $TWITTER_BEARER_TOKEN" \
  "https://api.twitter.com/2/tweets?ids=ID1,ID2,ID3&tweet.fields=public_metrics"
```
Response includes: `retweet_count`, `reply_count`, `like_count`, `quote_count`, `bookmark_count`, `impression_count`

### Rate Limits
| Endpoint | Limit | Window |
|----------|-------|--------|
| POST /tweets | 300 tweets | 3 hours |
| DELETE /tweets | 50 deletes | 15 minutes |
| POST /likes | 50 likes | 15 minutes |
| GET /mentions | 180 requests | 15 minutes |
| GET /search/recent | 180 requests | 15 minutes |

Always check response headers:
- `x-rate-limit-limit`: Total requests allowed
- `x-rate-limit-remaining`: Requests remaining
- `x-rate-limit-reset`: Unix timestamp when limit resets

---

## Algorithm Optimization

The following signals are known to affect distribution as of 2024-2025. Treat as heuristics -- validate against your own account's data.

### Signals That Boost Distribution
| Signal | Why It Matters | How to Leverage |
|--------|---------------|-----------------|
| Early engagement (first 30 min) | Algorithm tests tweets on a small audience first; high early engagement triggers wider distribution | Post when your audience is most active; craft strong hooks |
| Dwell time | Time spent reading your tweet/thread counts as engagement | Write threads (keeps users scrolling), use line breaks for readability |
| Replies (especially conversations) | Reply chains signal valuable content | End tweets with questions; reply to your own replies to keep threads going |
| Bookmarks/saves | Strong quality signal (user wants to return) | Post actionable content (how-tos, frameworks, checklists) worth saving |
| Profile visits after viewing | Indicates your content made someone curious about you | Ensure your bio clearly states your expertise and value prop |

### Signals That Suppress Distribution
| Signal | Impact | Avoidance |
|--------|--------|-----------|
| External links in tweet body | Reduced impressions (Twitter wants users on-platform) | Post the content natively; put links in a reply |
| Hashtag spam (3+) | Triggers spam filters | Use 0-2 relevant hashtags maximum |
| Rapid-fire posting | Floods follower timelines, reduces per-tweet engagement | Space posts 2-3 hours apart minimum |
| Low engagement ratio | Tweets with many impressions but no interaction signal low quality | Delete or don't repeat content formats that consistently underperform |
| Engagement bait without substance | "Like if you agree" without actual content | Pair CTAs with genuine value |

### Algorithm-Aware Posting Strategy
1. **Test before committing**: Post a single tweet on a topic. If engagement is above-average in 1 hour, follow up with a thread within 24 hours.
2. **Reply to yourself**: Add a reply with a link or context. This creates a conversation thread that boosts the original.
3. **Engagement window**: Reply to comments on your tweets within the first hour. Reply chains are rewarded.
4. **Content recycling**: A tweet that performed well 3+ months ago can be reposted with fresh wording.

---

## Media Upload Handling

### Twitter API v2 Media Upload
Twitter API v2 has no media upload endpoint. Use the v1.1 endpoint, which requires OAuth 1.0a.

**Simple upload (images < 5MB, GIFs < 15MB)**:
```bash
curl -X POST "https://upload.twitter.com/1.1/media/upload.json" \
  -H "Authorization: OAuth oauth_consumer_key=...,oauth_token=...,oauth_signature=..." \
  -F "media=@/path/to/image.png"
```
Response: `{"media_id": 123456789, "media_id_string": "123456789"}`

**Attach media to a tweet**:
```bash
curl -s -X POST "https://api.twitter.com/2/tweets" \
  -H "Authorization: Bearer $TWITTER_BEARER_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"text": "Check this out", "media": {"media_ids": ["123456789"]}}'
```

**Alt text for accessibility** (set after upload, before tweeting):
```bash
curl -X POST "https://upload.twitter.com/1.1/media/metadata/create.json" \
  -H "Authorization: OAuth ..." \
  -H "Content-Type: application/json" \
  -d '{"media_id": "123456789", "alt_text": {"text": "Description of the image"}}'
```

### When to Use Media
- **Data/stats tweets**: Chart or highlighted number as image -- 2-3x more impressions
- **Thread hooks**: Image in tweet 1 increases click-through
- **Code snippets**: Screenshot with syntax highlighting beats plain text
- **Before/after**: Visual comparisons are highly shareable

### When to Skip Media
- OAuth 1.0a credentials not configured (Bearer Token alone cannot upload)
- Image does not add information beyond the text
- Attaching media would delay posting past the optimal window

---

## Content Strategy Framework

### Content Pillars
Define 3-5 core topics ("pillars") that all content revolves around:
```
Example for a tech founder:
  Pillar 1: AI & Machine Learning (40% of content)
  Pillar 2: Startup Building (30% of content)
  Pillar 3: Engineering Culture (20% of content)
  Pillar 4: Personal Growth (10% of content)
```

### Content Mix (7 types)
| Type | Frequency | Purpose | Template |
|------|-----------|---------|----------|
| Hot take | 2-3/week | Engagement | "Unpopular opinion: [contrarian view]" |
| Thread | 1-2/week | Authority | "I spent X hours researching Y. Here's what I found:" |
| Tip/How-to | 2-3/week | Value | "How to [solve problem] in [N] steps:" |
| Question | 1-2/week | Engagement | "[Interesting question]? I'll go first:" |
| Curated share | 1-2/week | Curation | "This [article/tool/repo] is a game changer for [audience]:" |
| Story | 1/week | Connection | "3 years ago I [relatable experience]. Here's what happened:" |
| Data/Stat | 1/week | Authority | "[Surprising statistic]. Here's why it matters:" |

### Optimal Posting Times (UTC-based, adjust to audience timezone)
| Day | Best Times | Why |
|-----|-----------|-----|
| Monday | 8-10 AM | Start of work week, checking feeds |
| Tuesday | 10 AM, 1 PM | Peak engagement day |
| Wednesday | 9 AM, 12 PM | Mid-week focus |
| Thursday | 10 AM, 2 PM | Second-highest engagement day |
| Friday | 9-11 AM | Morning only, engagement drops PM |
| Saturday | 10 AM | Casual browsing |
| Sunday | 4-6 PM | Pre-work-week planning |

---

## Tweet Writing Best Practices

### The Hook (first line is everything)
Hooks that work:
- **Contrarian**: "Most people think X. They're wrong."
- **Number**: "I analyzed 500 [things]. Here's what I found:"
- **Question**: "Why do 90% of [things] fail?"
- **Story**: "In 2019, I almost [dramatic thing]."
- **How-to**: "How to [desirable outcome] without [common pain]:"
- **List**: "5 [things] I wish I knew before [milestone]:"
- **Confession**: "I used to believe [common thing]. Then I learned..."

### Writing Rules
1. **One idea per tweet** — don't try to cover everything
2. **Front-load value** — the hook must deliver or promise value
3. **Use line breaks** — no wall of text, 1-2 sentences per line
4. **280 character limit** — every word must earn its place
5. **Active voice** — "We shipped X" not "X was shipped by us"
6. **Specific > vague** — "3x faster" not "much faster"
7. **End with a call to action** — "Agree? RT" or "What would you add?"

### Thread Structure
```
Tweet 1 (HOOK): Compelling opening that makes people click "Show this thread"
  - Must stand alone as a great tweet
  - End with "A thread:" or "Here's what I found:"

Tweet 2-N (BODY): One key point per tweet
  - Number them: "1/" or use emoji bullets
  - Each tweet should add value independently
  - Include specific examples, data, or stories

Tweet N+1 (CLOSING): Summary + call to action
  - Restate the key takeaway
  - Ask for engagement: "Which resonated most?"
  - Self-reference: "If this was useful, follow @handle for more"
```

### Hashtag Strategy
- **0-2 hashtags** per tweet (more looks spammy)
- Use hashtags for discovery, not decoration
- Mix broad (#AI) and specific (#LangChain)
- Never use hashtags in threads (except maybe tweet 1)
- Research trending hashtags in your niche before using them

---

## Engagement Playbook

### Replying to Mentions
Rules:
1. **Respond within 2 hours** during engagement_hours
2. **Add value** — don't just say "thanks!" — expand on their point
3. **Ask a follow-up question** — drives conversation
4. **Be genuine** — match their energy and tone
5. **Never argue** — if someone is hostile, ignore or block

Reply templates:
- Agreement: "Great point! I'd also add [related insight]"
- Question: "Interesting question. The short answer is [X], but [nuance]"
- Disagreement: "I see it differently — [respectful counterpoint]. What's your experience?"
- Gratitude: "Appreciate you sharing this! [Specific thing you liked about their tweet]"

### When NOT to Engage
- Trolls or obviously bad-faith arguments
- Political flame wars (unless that's your content pillar)
- Personal attacks (block immediately)
- Spam or bot accounts
- Tweets that could create legal liability

### Auto-Like Strategy
Like tweets from:
1. People who regularly engage with your content (reciprocity)
2. Influencers in your niche (visibility)
3. Thoughtful content related to your pillars (curation signal)
4. Replies to your tweets (encourages more replies)

Do NOT auto-like:
- Controversial or political content
- Content you haven't actually read
- Spam or low-quality threads
- Competitor criticism (looks petty)

---

## Advanced Engagement Patterns

### Quote Tweet vs Reply vs Retweet

Choosing the right interaction type determines whether you gain visibility or waste it.

**Use a Quote Tweet when**:
- You have a distinct take or added context (not just "this!")
- The original tweet has high impressions and you want to draft off its reach
- You are crediting someone while adding your own insight for your audience
- The original author has a similar or larger following (exposes you to their audience)

**Use a Reply when**:
- You want to build a direct relationship with the author
- Your comment only makes sense in context of the original
- The original author has a much larger following (replies show on their thread, giving you visibility without looking self-promotional)
- You are answering a question or adding a correction

**Use a plain Retweet when**:
- The original says everything perfectly and you have nothing to add
- You want to signal-boost a community member, customer, or partner
- The content is time-sensitive (breaking news, event announcements)

**Avoid**:
- Quote tweeting with only emojis or "this" -- adds no value, looks lazy
- Quote tweeting someone with fewer followers just to dunk -- punching down
- Retweeting more than 3-4 times per day -- dilutes your original content ratio

### Thread Repurposing

A thread that performed well contains 5-7 standalone content pieces. Extract them over the following week to maximize ROI.

**Process**:
1. Day 0 (original): Post the full thread
2. Day 2: Pull the single most quotable tweet from the thread. Post it standalone with slightly different wording. No link back to the thread
3. Day 4: Turn a data point or example from the thread into a graphic or screenshot tweet
4. Day 6: Post the thread's core thesis as a hot take (one tweet, punchy)
5. Day 8+: If engagement stayed strong, post a "Part 2" thread that goes deeper on whichever tweet in the original got the most replies

**Rules**:
- Change the wording each time -- copy-pasting feels like spam to followers who saw the original
- Space extractions at least 48 hours apart
- Stop if any extraction underperforms significantly -- the topic is tapped out
- Never repurpose a thread that got low engagement; the content did not resonate

### Trending Topic Participation

**When to participate**:
- The trend directly intersects one of your content pillars
- You have a genuine, informed perspective (not a generic reaction)
- The trend is still rising (check the "Trending" tab; if it has been trending for >12 hours, you are late)
- The tone of the trend matches your brand voice

**When to avoid**:
- Tragedy, disaster, or crisis events -- opportunistic posting destroys trust
- Highly polarized political or social debates outside your expertise
- Trends driven by outrage mobs -- associating your brand is high-risk, low-reward
- You would need to force-fit your product or message into the trend

**Execution**:
- Lead with your actual insight, not the hashtag. The hashtag goes at the end or is omitted entirely if the topic keyword is in your text
- Be early or be different. If 50 people have already made the same joke, skip it
- Tie back to your pillar: "Trend X is exactly why [your pillar topic] matters more than ever"

---

## Crisis & Negative Comment Management

### Classifying Negative Interactions

Not all negative replies require the same response. Classify before acting:

| Type | Example | Action |
|------|---------|--------|
| **Constructive criticism** | "Your benchmark methodology is flawed because X" | Reply with acknowledgment, address the specific point, thank them |
| **Frustrated user** | "I tried your tool and it broke on my setup" | Reply publicly with empathy, ask for details, move to DMs if needed |
| **Trolling** | Personal insults, bad-faith arguments, bait | Do not reply. Block if repeated. Never quote-tweet to "expose" them |
| **Misinformation about you** | Factually wrong claims about your product/work | Reply once with facts and evidence. Do not engage further if they persist |
| **Pile-on / ratio** | Many negative replies at once, often from outside your audience | Pause all posting. Do not delete the original tweet (looks like hiding). Wait 24 hours before responding |

### Response Templates
- **Constructive criticism**: "Fair point — [acknowledgment]. We actually [explanation]. Appreciate you raising this."
- **Frustrated user**: "Sorry you hit that. Can you share [detail]? Happy to help sort it out."
- **Factual correction**: "To clarify — [correct info with source]. Happy to discuss further."

### Rules During a Crisis
1. **Stop all scheduled posts immediately** — auto-posting during a crisis looks tone-deaf
2. **Do not delete** the original tweet unless it contains genuinely harmful misinformation
3. **Acknowledge** the situation in a single, clear tweet if it involves your product/brand
4. **Do not be defensive** — own mistakes directly
5. **Wait before responding** — draft a response and review it after 1 hour
6. **Resume normal posting** only after the situation has cooled down (24-48 hours minimum)

---

## Content Calendar Template

```
WEEK OF [DATE]

Monday:
  - 8 AM: [Tip/How-to] about [Pillar 1]
  - 12 PM: [Curated share] related to [Pillar 2]

Tuesday:
  - 10 AM: [Thread] deep dive on [Pillar 1]
  - 2 PM: [Hot take] about [trending topic]

Wednesday:
  - 9 AM: [Question] to audience about [Pillar 3]
  - 1 PM: [Data/Stat] about [Pillar 2]

Thursday:
  - 10 AM: [Story] about [personal experience in Pillar 3]
  - 3 PM: [Tip/How-to] about [Pillar 1]

Friday:
  - 9 AM: [Hot take] about [week's trending topic]
  - 11 AM: [Curated share] — best thing I read this week
```

---

## Worked Examples

### Example 1: Product Launch Twitter Campaign (1-Week Plan)

**Context**: A dev tools startup is launching "FastDB," an open-source embedded database. The account has 2,400 followers, mostly backend engineers.

**Pre-launch (3 days before)**:
- Seed curiosity without revealing the product name
- Engage heavily in database-related threads to increase profile visits before launch

**Day 1 (Monday) -- Teaser**:
```
We've been heads-down for 8 months building something
we think embedded databases have been missing.

Shipping it open-source this Thursday.

More soon.
```
Purpose: Create anticipation. No hashtags, no links. Let curiosity drive profile visits.

**Day 2 (Tuesday) -- Problem framing**:
```
SQLite is incredible for what it does.

But if you need concurrent writes, ACID transactions,
AND sub-millisecond reads in the same embedded DB...
your options get thin fast.

We've been living in that gap. Fix incoming Thursday.
```
Purpose: Define the problem space. People who feel this pain will follow for the reveal.

**Day 3 (Wednesday) -- Social proof / build-up**:
```
Shipped our embedded DB to 12 beta testers last month.

Results so far:
- 4.2x faster concurrent writes vs SQLite WAL mode
- Zero-config replication
- Single static binary, 3.8 MB

One more day.
```
Purpose: Concrete numbers build credibility. "One more day" maintains tension.

**Day 4 (Thursday) -- Launch day thread** (6-tweet thread):
```
1/6 [HOOK]: Introducing FastDB -- embedded DB for concurrent-write-heavy
     workloads. Open source. Single binary. Here's why we built it:
2/6 [PROBLEM]: SQLite = single-writer. Fine for reads, hits a wall on
     write-heavy apps (event logging, IoT, realtime sync). FastDB uses
     MVCC -- writers never block readers, readers never block writers.
3/6 [PROOF]: Benchmarks (M2 Mac, 8 threads): concurrent writes 51K ops/s
     vs SQLite WAL 12K ops/s. Point reads on par at ~900K ops/s.
4/6 [ONBOARD]: Getting started: `cargo add fastdb` then 3 lines of code.
     Full SQLite-compatible query layer coming in v0.2.
5/6 [ROADMAP]: v0.1 ships ACID transactions, built-in replication, crash
     recovery, zero deps beyond libc. v0.2: SQL layer, S3 cold storage.
6/6 [CTA]: Star the repo: github.com/example/fastdb -- open issues, roast
     the benchmarks, tell us what's missing.
```
Key structural choices: tweet 1 is a standalone hook, tweet 3 has hard numbers, tweet 6 ends with a specific ask (not just "check it out").

**Day 4 afternoon** -- Post a standalone tweet answering the most common reply question publicly (drives docs traffic). **Day 5 (Friday)** -- Reply to every substantive comment. Templates for common reactions:
- "How is this different from X?" -> Concrete comparison, link to docs
- "Benchmarks look suspicious" -> Link the reproduction steps, invite them to run it
- "Will you support [feature]?" -> Link the tracking issue

**Day 6-7 (Weekend)** -- Repurpose: extract the benchmark tweet as a standalone with a chart image; post a "5 things I learned launching an open-source DB" reflection thread.

### Example 2: Building Thought Leadership from Scratch (Month 1)

**Context**: An individual ML engineer with 180 followers wants to become a recognized voice in applied machine learning. No existing audience. No viral content history.

**Core principle for month 1**: Do not broadcast. Contribute. Your first 500 followers come from being consistently useful in other people's threads, not from your own tweets.

**Week 1 -- Comment-first growth**:
- Post 0 original tweets
- Find 10 accounts in your niche with 5K-50K followers who post regularly
- Reply to 5-8 of their tweets per day with substantive comments (not "great post!")
- Goal: Get 3-5 of those authors to like or reply to your comments by end of week

**What a good reply looks like**:
```
Original tweet: "Fine-tuning LLMs is overrated. Most use cases
are better served by good prompting + RAG."

Bad reply: "Agreed!"

Good reply: "Mostly agree, but there's a middle ground --
LoRA fine-tuning on 500 domain-specific examples
consistently beats RAG for structured extraction tasks.

We saw 23% higher F1 on invoice parsing after a 2-hour
fine-tune vs our best RAG setup.

RAG still wins for open-domain QA though."
```
This reply adds data, shows experience, and invites further discussion. People reading the thread see your expertise and check your profile.

**Week 2 -- First original content**:
- Continue the reply strategy (5/day minimum)
- Post 2-3 original tweets. Keep them observational, not promotional:
```
Something I've noticed after fine-tuning 30+ models
this year:

The quality of your eval set matters 10x more than
the size of your training set.

50 carefully labeled examples with clear edge cases
beats 5000 noisy scraped examples every time.
```
- Post 1 "ask the audience" tweet to start conversations:
```
ML engineers: what's the most counterintuitive lesson
you've learned about deploying models to production?

I'll start: the model is almost never the bottleneck.
Data pipelines are.
```

**Week 3 -- First thread** (5-tweet authority thread):
```
1/5 [HOOK]: I've deployed 12 ML models to production this year. The ones
     that worked all had one thing in common. It wasn't the architecture.
2/5 [THESIS]: Every success had a tight feedback loop -- predictions
     validated by a human within 24 hours, not "we'll evaluate next quarter."
3/5 [EVIDENCE]: Model A (invoice classifier): accountants flagged errors
     same-day, retrained weekly, 84% -> 97% in 6 weeks. Model B (churn
     predictor): sales ignored outputs, no feedback 3 months, drifted to
     coin-flip accuracy.
4/5 [FRAMEWORK]: The pattern: (1) deploy with human-in-the-loop review,
     (2) log every correction, (3) retrain on corrections every 1-2 weeks,
     (4) remove human review once accuracy stabilizes.
5/5 [CTA]: If you're skipping the feedback loop, you're building on sand.
     What's your experience?
```
Notice the structure: personal credibility in tweet 1, a clear thesis in tweet 2, contrasting real examples in tweet 3, an actionable takeaway in tweet 4, and a discussion prompt in tweet 5.

**Week 4 -- Establish rhythm**:
- Settle into a sustainable cadence: 1 thread/week, 1-2 standalone tweets/day, 5+ replies/day
- Review metrics from week 2-3 content to identify which topics resonated
- Double down on the topic that got the most replies (not likes -- replies indicate deeper engagement)

**Month 1 milestones**:
| Metric | Target | Why it matters |
|--------|--------|----------------|
| Followers | 350-500 | 2-3x growth signals the approach is working |
| Avg impressions per tweet | 800-2000 | Shows the algorithm is distributing your content |
| Replies received per original tweet | 3-5 | People are engaging, not just scrolling past |
| Mutual follows from target accounts | 5-10 | Your niche peers are noticing you |
| Profile visits / week | 200+ | Your replies are driving curiosity |

**What to avoid in month 1**:
- Posting 10 tweets/day hoping something sticks -- looks desperate, exhausts your ideas
- Buying followers or using engagement pods -- Twitter's algorithm detects and penalizes this
- Talking about yourself or your product -- earn attention through insight first
- Getting discouraged by low numbers -- 180 to 400 followers in a month is strong growth

---

## Performance Metrics

### Key Metrics
| Metric | What It Measures | Good Benchmark |
|--------|-----------------|----------------|
| Impressions | How many people saw the tweet | Varies by follower count |
| Engagement rate | (likes+RTs+replies)/impressions | >2% is good, >5% is great |
| Reply rate | replies/impressions | >0.5% is good |
| Retweet rate | RTs/impressions | >1% is good |
| Profile visits | People checking your profile after tweet | Track trend |
| Follower growth | Net new followers per period | Track trend |

### Engagement Rate Formula
```
engagement_rate = (likes + retweets + replies + quotes) / impressions * 100

Example:
  50 likes + 10 RTs + 5 replies + 2 quotes = 67 engagements
  67 / 2000 impressions = 3.35% engagement rate
```

### Content Performance Analysis
Track which content types and topics perform best:
```
| Content Type | Avg Impressions | Avg Engagement Rate | Best Performing |
|-------------|-----------------|--------------------|--------------------|
| Hot take | 2500 | 4.2% | "Unpopular opinion: ..." |
| Thread | 5000 | 3.1% | "I analyzed 500 ..." |
| Tip | 1800 | 5.5% | "How to ... in 3 steps" |
```

Use this data to optimize future content mix.

---

## Brand Voice Guide

### Voice Dimensions
| Dimension | Range | Description |
|-----------|-------|-------------|
| Formal ↔ Casual | 1-5 | 1=corporate, 5=texting a friend |
| Serious ↔ Humorous | 1-5 | 1=all business, 5=comedy account |
| Reserved ↔ Bold | 1-5 | 1=diplomatic, 5=no-filter |
| General ↔ Technical | 1-5 | 1=anyone can understand, 5=deep expert |

### Consistency Rules
- Use the same voice across ALL tweets (hot takes and how-tos)
- Develop 3-5 "signature phrases" you reuse naturally
- If the brand voice says "casual," don't suddenly write a formal thread
- Read tweets aloud — does it sound like the same person?

---

## Safety & Compliance

### Content Guidelines
NEVER post:
- Discriminatory content (race, gender, religion, sexuality, disability)
- Defamatory claims about real people or companies
- Private or confidential information
- Threats, harassment, or incitement to violence
- Impersonation of other accounts
- Misleading claims presented as fact
- Content that violates Twitter Terms of Service

### Approval Mode Queue Format
```json
[
  {
    "id": "q_001",
    "content": "Tweet text here",
    "type": "hot_take",
    "pillar": "AI",
    "scheduled_for": "2025-01-15T10:00:00Z",
    "created": "2025-01-14T20:00:00Z",
    "status": "pending",
    "notes": "Based on trending discussion about LLM pricing"
  }
]
```

Preview file for human review:
```markdown
# Tweet Queue Preview
Generated: YYYY-MM-DD

## Pending Tweets (N total)

### 1. [Hot Take] — Scheduled: Mon 10 AM
> Tweet text here

**Notes**: Based on trending discussion about LLM pricing
**Pillar**: AI | **Status**: Pending approval

---

### 2. [Thread] — Scheduled: Tue 10 AM
> Tweet 1/5: Hook text here
> Tweet 2/5: Point one
> ...

**Notes**: Deep dive on new benchmark results
**Pillar**: AI | **Status**: Pending approval
```

### Risk Assessment
Before posting, evaluate each tweet:
- Could this be misinterpreted? → Rephrase for clarity
- Does this punch down? → Don't post
- Would you be comfortable seeing this attributed to the user in a news article? → If no, don't post
- Is this verifiably true? → If not sure, add hedging language or don't post
