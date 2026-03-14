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

**Step 1: Get access token**:
```bash
curl -s -X POST "https://www.reddit.com/api/v1/access_token" \
  -u "$REDDIT_CLIENT_ID:$REDDIT_CLIENT_SECRET" \
  -d "grant_type=password&username=$REDDIT_USERNAME&password=$REDDIT_PASSWORD" \
  -A "LibreFang Reddit Hand/1.0"
```
Response: `{"access_token": "...", "token_type": "bearer", "expires_in": 86400, "scope": "*"}`

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
| With user-level auth | 30 requests | 1 minute |
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
