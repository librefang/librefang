# Constant Contact Routing Reference

**App name:** `constant-contact`
**Base URL proxied:** `api.cc.email`

## API Path Pattern

```
/constant-contact/v3/{resource}
```

## Common Endpoints

### Account

#### Get Account Summary
```bash
GET /constant-contact/v3/account/summary
```

#### Update Account Summary
```bash
PUT /constant-contact/v3/account/summary
Content-Type: application/json

{
  "first_name": "John",
  "last_name": "Doe",
  "organization_name": "Acme Inc"
}
```

#### Get Account Emails
```bash
GET /constant-contact/v3/account/emails
```

#### Add Account Email
```bash
POST /constant-contact/v3/account/emails
Content-Type: application/json

{
  "email_address": "newsender@example.com"
}
```

#### Get User Privileges
```bash
GET /constant-contact/v3/account/user/privileges
```

### Contacts

#### List Contacts
```bash
GET /constant-contact/v3/contacts
GET /constant-contact/v3/contacts?email=john@example.com&status=all
GET /constant-contact/v3/contacts?include=custom_fields,list_memberships,taggings&limit=50
GET /constant-contact/v3/contacts?updated_after=2026-04-01T00:00:00Z
```

#### Get Contact
```bash
GET /constant-contact/v3/contacts/{contact_id}
GET /constant-contact/v3/contacts/{contact_id}?include=custom_fields,list_memberships,taggings,notes
```

#### Create Contact

Requires `create_source` field:

```bash
POST /constant-contact/v3/contacts
Content-Type: application/json

{
  "email_address": {
    "address": "john@example.com",
    "permission_to_send": "implicit"
  },
  "first_name": "John",
  "last_name": "Doe",
  "create_source": "Account",
  "list_memberships": ["list-uuid"]
}
```

#### Update Contact

Requires `update_source` field:

```bash
PUT /constant-contact/v3/contacts/{contact_id}
Content-Type: application/json

{
  "email_address": {"address": "john@example.com"},
  "first_name": "John",
  "last_name": "Smith",
  "update_source": "Account"
}
```

#### Delete Contact
```bash
DELETE /constant-contact/v3/contacts/{contact_id}
```

#### Create or Update (Sign-Up Form)
```bash
POST /constant-contact/v3/contacts/sign_up_form
Content-Type: application/json

{
  "email_address": "john@example.com",
  "first_name": "John",
  "last_name": "Doe",
  "list_memberships": ["list-uuid"]
}
```

#### Get Contact Counts
```bash
GET /constant-contact/v3/contacts/counts
```

### Contact Lists

#### List Contact Lists
```bash
GET /constant-contact/v3/contact_lists
GET /constant-contact/v3/contact_lists?include_membership_count=all
```

#### Get Contact List
```bash
GET /constant-contact/v3/contact_lists/{list_id}
```

#### Create Contact List
```bash
POST /constant-contact/v3/contact_lists
Content-Type: application/json

{
  "name": "Newsletter Subscribers",
  "description": "Main newsletter list",
  "favorite": false
}
```

#### Update Contact List
```bash
PUT /constant-contact/v3/contact_lists/{list_id}
Content-Type: application/json

{
  "name": "Updated List Name",
  "description": "Updated description",
  "favorite": true
}
```

#### Delete Contact List
```bash
DELETE /constant-contact/v3/contact_lists/{list_id}
```

### Tags

#### List Tags
```bash
GET /constant-contact/v3/contact_tags
```

#### Create Tag
```bash
POST /constant-contact/v3/contact_tags
Content-Type: application/json

{
  "name": "VIP Customer"
}
```

#### Update Tag
```bash
PUT /constant-contact/v3/contact_tags/{tag_id}
Content-Type: application/json

{
  "name": "Premium Customer"
}
```

#### Delete Tag
```bash
DELETE /constant-contact/v3/contact_tags/{tag_id}
```

### Custom Fields

#### List Custom Fields
```bash
GET /constant-contact/v3/contact_custom_fields
```

#### Create Custom Field
```bash
POST /constant-contact/v3/contact_custom_fields
Content-Type: application/json

{
  "label": "Customer ID",
  "type": "string"
}
```

#### Delete Custom Field
```bash
DELETE /constant-contact/v3/contact_custom_fields/{custom_field_id}
```

### Email Campaigns

#### List Email Campaigns
```bash
GET /constant-contact/v3/emails
GET /constant-contact/v3/emails?limit=50&after_date=2026-01-01T00:00:00Z
```

#### Get Email Campaign
```bash
GET /constant-contact/v3/emails/{campaign_id}
```

#### Create Email Campaign
```bash
POST /constant-contact/v3/emails
Content-Type: application/json

{
  "name": "March Newsletter",
  "email_campaign_activities": [
    {
      "format_type": 5,
      "from_name": "Company",
      "from_email": "marketing@example.com",
      "reply_to_email": "reply@example.com",
      "subject": "Newsletter",
      "html_content": "<html><body>Hello</body></html>"
    }
  ]
}
```

#### Rename Email Campaign
```bash
PATCH /constant-contact/v3/emails/{campaign_id}
Content-Type: application/json

{
  "name": "New Campaign Name"
}
```

#### Delete Email Campaign
```bash
DELETE /constant-contact/v3/emails/{campaign_id}
```

### Email Campaign Activities

#### Get Campaign Activity
```bash
GET /constant-contact/v3/emails/activities/{campaign_activity_id}
```

#### Update Campaign Activity
```bash
PUT /constant-contact/v3/emails/activities/{campaign_activity_id}
Content-Type: application/json

{
  "from_name": "Company",
  "from_email": "marketing@example.com",
  "reply_to_email": "reply@example.com",
  "subject": "Updated Subject",
  "html_content": "<html><body>Updated</body></html>",
  "contact_list_ids": ["list-uuid"]
}
```

#### Preview Campaign Activity
```bash
GET /constant-contact/v3/emails/activities/{campaign_activity_id}/previews
```

#### Send Test Email
```bash
POST /constant-contact/v3/emails/activities/{campaign_activity_id}/tests
Content-Type: application/json

{
  "email_addresses": ["test@example.com"]
}
```

#### Schedule Campaign
```bash
POST /constant-contact/v3/emails/activities/{campaign_activity_id}/schedules
Content-Type: application/json

{
  "scheduled_date": "2026-06-01T10:00:00Z"
}
```

#### Get Campaign Schedule
```bash
GET /constant-contact/v3/emails/activities/{campaign_activity_id}/schedules
```

#### Unschedule Campaign
```bash
DELETE /constant-contact/v3/emails/activities/{campaign_activity_id}/schedules
```

### Segments

#### List Segments
```bash
GET /constant-contact/v3/segments
```

#### Get Segment
```bash
GET /constant-contact/v3/segments/{segment_id}
```

#### Create Segment
```bash
POST /constant-contact/v3/segments
Content-Type: application/json

{
  "name": "Engaged Subscribers",
  "segment_criteria": { ... }
}
```

#### Delete Segment
```bash
DELETE /constant-contact/v3/segments/{segment_id}
```

### Bulk Activities

#### List Activities
```bash
GET /constant-contact/v3/activities
```

#### Get Activity Status
```bash
GET /constant-contact/v3/activities/{activity_id}
```

#### Add Contacts to Lists
```bash
POST /constant-contact/v3/activities/add_list_memberships
Content-Type: application/json

{
  "source": {"contact_ids": ["uuid-1", "uuid-2"]},
  "list_ids": ["list-uuid"]
}
```

#### Remove Contacts from Lists
```bash
POST /constant-contact/v3/activities/remove_list_memberships
Content-Type: application/json

{
  "source": {"contact_ids": ["uuid-1"]},
  "list_ids": ["list-uuid"]
}
```

#### Add Tags to Contacts
```bash
POST /constant-contact/v3/activities/contacts_taggings_add
Content-Type: application/json

{
  "source": {"contact_ids": ["uuid-1"]},
  "tag_ids": ["tag-uuid"]
}
```

#### Remove Tags from Contacts
```bash
POST /constant-contact/v3/activities/contacts_taggings_remove
Content-Type: application/json

{
  "source": {"contact_ids": ["uuid-1"]},
  "tag_ids": ["tag-uuid"]
}
```

#### Export Contacts
```bash
POST /constant-contact/v3/activities/contact_exports
Content-Type: application/json

{
  "contact_ids": ["uuid-1"],
  "fields": ["first_name", "last_name", "email"]
}
```

#### Download Export
```bash
GET /constant-contact/v3/contact_exports/{export_id}
```

#### Delete Contacts in Bulk
```bash
POST /constant-contact/v3/activities/contact_delete
Content-Type: application/json

{
  "contact_ids": ["uuid-1", "uuid-2"]
}
```

### Reporting

#### Email Campaign Summaries
```bash
GET /constant-contact/v3/reports/summary_reports/email_campaign_summaries
```

#### Get Email Campaign Report
```bash
GET /constant-contact/v3/reports/email_reports/{campaign_activity_id}
```

#### Contact Activity Summary
```bash
GET /constant-contact/v3/reports/contact_reports/{contact_id}/activity_summary
```

## Pagination

Cursor-based pagination using `limit` and `cursor` parameters:

```bash
GET /constant-contact/v3/contacts?limit=50
```

Response includes:
```json
{
  "contacts": [...],
  "_links": {
    "next": {
      "href": "/v3/contacts?cursor=abc123"
    }
  }
}
```

Use `cursor` for next page:
```bash
GET /constant-contact/v3/contacts?cursor=abc123
```

## Notes

- Authentication is automatic - the router injects the OAuth token
- Resource IDs use UUID format (36 characters with hyphens)
- All dates use ISO-8601 format
- `create_source` is required for contact creation; `update_source` for updates
- `from_email` must be a confirmed account email address
- Bulk operations are asynchronous - poll activity status for completion
- Tags and lists return `202 Accepted` on delete (async); contacts and campaigns return `204 No Content`
- Maximum 1,000 contact lists per account
- A contact can belong to up to 50 lists

## Resources

- [V3 API Overview](https://developer.constantcontact.com/api_guide/getting_started.html)
- [API Reference](https://developer.constantcontact.com/api_reference/index.html)
- [Technical Overview](https://developer.constantcontact.com/api_guide/v3_technical_overview.html)
