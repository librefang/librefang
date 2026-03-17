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
1. **Dwell time**: How long people spend reading your post
2. **Early engagement**: Comments in the first hour boost distribution
3. **Meaningful comments**: Long comments signal quality content
4. **No external links**: Posts with links get 40-50% less reach
5. **Personal stories**: Narrative content outperforms promotional content

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
