# GoHighLevel (PIT) Routing Reference

**App name:** `highlevel-pit`
**Base URL proxied:** `services.leadconnectorhq.com`

## API Path Pattern

```
/highlevel-pit/{resource}
```

## Two Token Types

GoHighLevel uses Agency tokens and Sub-Account tokens with different scopes:

- **Agency token**: Manage locations (sub-accounts), snapshots
- **Sub-Account token**: Contacts, calendars, pipelines, conversations, payments, custom fields, tags, workflows, campaigns

Use the `Maton-Connection` header to specify which token to use.

## Common Endpoints — Agency Token

### Search Locations
```bash
GET /highlevel-pit/locations/search?companyId={companyId}
```

### Get Location
```bash
GET /highlevel-pit/locations/{locationId}
```

### Create Location
```bash
POST /highlevel-pit/locations/
Content-Type: application/json

{
  "companyId": "{companyId}",
  "name": "New Sub-Account",
  "address": "123 Main St",
  "city": "San Francisco",
  "state": "CA",
  "country": "US",
  "timezone": "America/Los_Angeles",
  "email": "admin@example.com"
}
```

### Update Location
```bash
PUT /highlevel-pit/locations/{locationId}
Content-Type: application/json

{
  "name": "Updated Name"
}
```

### Delete Location
```bash
DELETE /highlevel-pit/locations/{locationId}
```

### List Snapshots
```bash
GET /highlevel-pit/snapshots/?companyId={companyId}
```

## Common Endpoints — Sub-Account Token

### Contacts

#### List Contacts
```bash
GET /highlevel-pit/contacts/?locationId={locationId}&limit=20
GET /highlevel-pit/contacts/?locationId={locationId}&query=john@example.com
```

#### Get Contact
```bash
GET /highlevel-pit/contacts/{contactId}
```

#### Create Contact
```bash
POST /highlevel-pit/contacts/
Content-Type: application/json

{
  "locationId": "{locationId}",
  "firstName": "John",
  "lastName": "Doe",
  "email": "john@example.com",
  "phone": "+15551234567",
  "tags": ["customer"]
}
```

#### Update Contact
```bash
PUT /highlevel-pit/contacts/{contactId}
Content-Type: application/json

{
  "firstName": "Jane"
}
```

#### Delete Contact
```bash
DELETE /highlevel-pit/contacts/{contactId}
```

### Contact Tags
```bash
POST /highlevel-pit/contacts/{contactId}/tags  — Add tags
DELETE /highlevel-pit/contacts/{contactId}/tags — Remove tags
```

### Contact Notes
```bash
GET /highlevel-pit/contacts/{contactId}/notes
POST /highlevel-pit/contacts/{contactId}/notes
PUT /highlevel-pit/contacts/{contactId}/notes/{noteId}
DELETE /highlevel-pit/contacts/{contactId}/notes/{noteId}
```

### Contact Tasks
```bash
GET /highlevel-pit/contacts/{contactId}/tasks
POST /highlevel-pit/contacts/{contactId}/tasks  — requires "completed" field
PUT /highlevel-pit/contacts/{contactId}/tasks/{taskId}
DELETE /highlevel-pit/contacts/{contactId}/tasks/{taskId}
```

### Opportunities
```bash
GET /highlevel-pit/opportunities/search?location_id={locationId}
GET /highlevel-pit/opportunities/{opportunityId}
POST /highlevel-pit/opportunities/
PUT /highlevel-pit/opportunities/{opportunityId}  — requires pipelineId
DELETE /highlevel-pit/opportunities/{opportunityId}
```

### Pipelines
```bash
GET /highlevel-pit/opportunities/pipelines?locationId={locationId}
```

### Calendars
```bash
GET /highlevel-pit/calendars/?locationId={locationId}
GET /highlevel-pit/calendars/{calendarId}
POST /highlevel-pit/calendars/
PUT /highlevel-pit/calendars/{calendarId}  — do NOT include locationId
DELETE /highlevel-pit/calendars/{calendarId}
GET /highlevel-pit/calendars/events?locationId={locationId}&calendarId={calendarId}&startTime={epochMs}&endTime={epochMs}
GET /highlevel-pit/calendars/{calendarId}/free-slots?startDate={epochMs}&endDate={epochMs}
GET /highlevel-pit/calendars/groups?locationId={locationId}
```

### Conversations
```bash
GET /highlevel-pit/conversations/search?locationId={locationId}
GET /highlevel-pit/conversations/{conversationId}
GET /highlevel-pit/conversations/{conversationId}/messages
POST /highlevel-pit/conversations/
```

### Users
```bash
GET /highlevel-pit/users/?locationId={locationId}
```

### Location Tags
```bash
GET /highlevel-pit/locations/{locationId}/tags
POST /highlevel-pit/locations/{locationId}/tags
GET /highlevel-pit/locations/{locationId}/tags/{tagId}
PUT /highlevel-pit/locations/{locationId}/tags/{tagId}
DELETE /highlevel-pit/locations/{locationId}/tags/{tagId}
```

### Custom Fields
```bash
GET /highlevel-pit/locations/{locationId}/customFields
POST /highlevel-pit/locations/{locationId}/customFields
GET /highlevel-pit/locations/{locationId}/customFields/{fieldId}
PUT /highlevel-pit/locations/{locationId}/customFields/{fieldId}
DELETE /highlevel-pit/locations/{locationId}/customFields/{fieldId}
```

### Custom Values
```bash
GET /highlevel-pit/locations/{locationId}/customValues
POST /highlevel-pit/locations/{locationId}/customValues
GET /highlevel-pit/locations/{locationId}/customValues/{valueId}
PUT /highlevel-pit/locations/{locationId}/customValues/{valueId}
DELETE /highlevel-pit/locations/{locationId}/customValues/{valueId}
```

### Businesses
```bash
GET /highlevel-pit/businesses/?locationId={locationId}
GET /highlevel-pit/businesses/{businessId}
POST /highlevel-pit/businesses/
PUT /highlevel-pit/businesses/{businessId}
DELETE /highlevel-pit/businesses/{businessId}
```

### Products
```bash
GET /highlevel-pit/products/?locationId={locationId}
GET /highlevel-pit/products/{productId}?locationId={locationId}
POST /highlevel-pit/products/
DELETE /highlevel-pit/products/{productId}?locationId={locationId}
```

### Invoices
```bash
GET /highlevel-pit/invoices/?altId={locationId}&altType=location&limit=20&offset=0
```

### Payments
```bash
GET /highlevel-pit/payments/orders?altId={locationId}&altType=location
GET /highlevel-pit/payments/transactions?altId={locationId}&altType=location
GET /highlevel-pit/payments/subscriptions?altId={locationId}&altType=location
```

### Trigger Links
```bash
GET /highlevel-pit/links/?locationId={locationId}
POST /highlevel-pit/links/
PUT /highlevel-pit/links/{linkId}
DELETE /highlevel-pit/links/{linkId}
```

### Other
```bash
GET /highlevel-pit/workflows/?locationId={locationId}
GET /highlevel-pit/campaigns/?locationId={locationId}
GET /highlevel-pit/forms/?locationId={locationId}
GET /highlevel-pit/surveys/?locationId={locationId}
GET /highlevel-pit/funnels/funnel/list?locationId={locationId}
GET /highlevel-pit/social-media-posting/{locationId}/accounts
GET /highlevel-pit/social-media-posting/{locationId}/categories
GET /highlevel-pit/medias/files?altId={locationId}&altType=location&type=file
```

## Notes

- Two token types with different scopes — use `Maton-Connection` header
- Most sub-account endpoints require `locationId` query parameter
- Payment/invoice endpoints use `altId` + `altType=location` instead of `locationId`
- Calendar events use epoch milliseconds for time parameters
- Calendar update must NOT include `locationId` in body
- Contact task creation requires `completed` boolean field
- Opportunity update requires `pipelineId` even when not changing it
- All delete operations return HTTP 200

## Resources

- [GoHighLevel API Documentation](https://highlevel.stoplight.io/docs/integrations/)
- [GoHighLevel Marketplace](https://marketplace.gohighlevel.com/docs/)
