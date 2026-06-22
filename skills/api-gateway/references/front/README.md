# Front Routing Reference

**App name:** `front`
**Base URL proxied:** `api2.frontapp.com`

## API Path Pattern

```
/front/{resource}
```

## Common Endpoints

### Company / Me

#### Get Current Company
```bash
GET /front/me
```

### Teammates

#### List Teammates
```bash
GET /front/teammates
```

#### Get Teammate
```bash
GET /front/teammates/{teammate_id}
```

### Teams

#### List Teams
```bash
GET /front/teams
```

### Inboxes

#### List Inboxes
```bash
GET /front/inboxes
```

#### Get Inbox
```bash
GET /front/inboxes/{inbox_id}
```

#### Create Inbox
```bash
POST /front/inboxes
Content-Type: application/json

{
  "name": "New Inbox",
  "teammate_ids": ["tea_abc123"]
}
```

### Channels

#### List Channels
```bash
GET /front/channels
```

#### Get Channel
```bash
GET /front/channels/{channel_id}
```

### Conversations

#### List Conversations
```bash
GET /front/conversations
GET /front/conversations?q=search_term
```

#### Get Conversation
```bash
GET /front/conversations/{conversation_id}
```

#### Update Conversation
```bash
PATCH /front/conversations/{conversation_id}
Content-Type: application/json

{
  "assignee_id": "tea_abc123",
  "status": "archived"
}
```

#### Update Assignee
```bash
PUT /front/conversations/{conversation_id}/assignee
Content-Type: application/json

{"assignee_id": "tea_abc123"}
```

### Messages

#### Get Message
```bash
GET /front/messages/{message_id}
```

#### Send Reply
```bash
POST /front/conversations/{conversation_id}/messages
Content-Type: application/json

{
  "author_id": "tea_abc123",
  "body": "Reply content",
  "type": "reply"
}
```

#### Send New Message
```bash
POST /front/channels/{channel_id}/messages
Content-Type: application/json

{
  "author_id": "tea_abc123",
  "to": ["recipient@example.com"],
  "subject": "Subject",
  "body": "Message body"
}
```

### Contacts

#### List Contacts
```bash
GET /front/contacts
GET /front/contacts?q=search_term
```

#### Get Contact
```bash
GET /front/contacts/{contact_id}
```

#### Create Contact
```bash
POST /front/contacts
Content-Type: application/json

{
  "name": "John Doe",
  "handles": [{"source": "email", "handle": "john@example.com"}]
}
```

#### Update Contact
```bash
PATCH /front/contacts/{contact_id}
Content-Type: application/json

{"name": "Updated Name"}
```

#### Delete Contact
```bash
DELETE /front/contacts/{contact_id}
```

### Tags

#### List Tags
```bash
GET /front/tags
```

#### Get Tag
```bash
GET /front/tags/{tag_id}
```

#### Create Tag
```bash
POST /front/tags
Content-Type: application/json

{
  "name": "Urgent",
  "highlight": "red"
}
```

#### Update Tag
```bash
PATCH /front/tags/{tag_id}
Content-Type: application/json

{"name": "Updated Tag"}
```

#### Delete Tag
```bash
DELETE /front/tags/{tag_id}
```

### Accounts

#### List Accounts
```bash
GET /front/accounts
```

#### Get Account
```bash
GET /front/accounts/{account_id}
```

#### Create Account
```bash
POST /front/accounts
Content-Type: application/json

{
  "name": "Acme Corp",
  "domains": ["acme.com"]
}
```

### Comments

#### List Conversation Comments
```bash
GET /front/conversations/{conversation_id}/comments
```

#### Create Comment
```bash
POST /front/conversations/{conversation_id}/comments
Content-Type: application/json

{
  "author_id": "tea_abc123",
  "body": "Internal note"
}
```

## Pagination

Cursor-based pagination:

```bash
GET /front/contacts?page_token={token}
```

Response includes:
```json
{
  "_pagination": {"next": "https://...?page_token=abc123"},
  "_results": [...]
}
```

## Notes

- Resource ID prefixes: `tea_` (teammate), `tim_` (team), `inb_` (inbox), `cha_` (channel), `cnv_` (conversation), `msg_` (message), `crd_` (contact), `tag_` (tag), `cmp_` (company)
- Timestamps are Unix timestamps (seconds)
- Responses include `_links` with related resource URLs
- Gateway proxies to company-specific subdomain

## Resources

- [Front API Reference](https://dev.frontapp.com/reference/introduction)
- [Front API Authentication](https://dev.frontapp.com/docs/authentication)
