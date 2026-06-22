# Twenty CRM Routing Reference

**App name:** `twenty`
**Base URL proxied:** `api.twenty.com`

## API Path Pattern

```
/twenty/rest/{resource}
```

## Common Endpoints

### Companies

#### List Companies
```bash
GET /twenty/rest/companies?limit=20
```

#### Get Company
```bash
GET /twenty/rest/companies/{id}
```

#### Create Company
```bash
POST /twenty/rest/companies
Content-Type: application/json

{
  "name": "Company Name",
  "domainName": {"primaryLinkUrl": "https://company.com"},
  "employees": 100
}
```

#### Update Company
```bash
PATCH /twenty/rest/companies/{id}
Content-Type: application/json

{
  "name": "Updated Name"
}
```

#### Delete Company
```bash
DELETE /twenty/rest/companies/{id}
```

### People

#### List People
```bash
GET /twenty/rest/people?limit=20
```

#### Get Person
```bash
GET /twenty/rest/people/{id}
```

#### Create Person
```bash
POST /twenty/rest/people
Content-Type: application/json

{
  "name": {"firstName": "John", "lastName": "Doe"},
  "emails": {"primaryEmail": "john@company.com"},
  "companyId": "{companyId}"
}
```

#### Update Person
```bash
PATCH /twenty/rest/people/{id}
Content-Type: application/json

{
  "jobTitle": "CEO"
}
```

### Opportunities

#### List Opportunities
```bash
GET /twenty/rest/opportunities?limit=20
```

#### Create Opportunity
```bash
POST /twenty/rest/opportunities
Content-Type: application/json

{
  "name": "Deal Name",
  "amount": {"amountMicros": 50000000000, "currencyCode": "USD"},
  "stage": "SCREENING",
  "companyId": "{companyId}"
}
```

### Notes

#### List Notes
```bash
GET /twenty/rest/notes?limit=20
```

#### Create Note
```bash
POST /twenty/rest/notes
Content-Type: application/json

{
  "title": "Note Title",
  "body": "Note content"
}
```

### Tasks

#### List Tasks
```bash
GET /twenty/rest/tasks?limit=20
```

#### Create Task
```bash
POST /twenty/rest/tasks
Content-Type: application/json

{
  "title": "Task Title",
  "status": "TODO",
  "dueAt": "2026-04-01T00:00:00.000Z"
}
```

### Workspace Members

#### List Workspace Members
```bash
GET /twenty/rest/workspaceMembers?limit=20
```

## Filtering

```bash
GET /twenty/rest/companies?filter=employees[gte]:100
GET /twenty/rest/opportunities?filter=stage[eq]:"MEETING"
```

Comparators: `eq`, `neq`, `gt`, `gte`, `lt`, `lte`, `in`, `is`, `like`, `ilike`, `startsWith`

## Pagination

Cursor-based pagination:

```bash
GET /twenty/rest/companies?limit=20&starting_after={endCursor}
```

Parameters:
- `limit` - Max 60 (default: 60)
- `starting_after` - Next page cursor
- `ending_before` - Previous page cursor

## Ordering

```bash
GET /twenty/rest/companies?order_by=createdAt[DescNullsLast]
```

Directions: `AscNullsFirst`, `AscNullsLast`, `DescNullsFirst`, `DescNullsLast`

## Notes

- All IDs are UUIDs
- Amount fields use micros (value × 1,000,000)
- Opportunity stages: SCREENING, MEETING, PROPOSAL, NEGOTIATION, WON, LOST
- Task statuses: TODO, IN_PROGRESS, DONE

## Resources

- [Twenty API Documentation](https://twenty.com/developers/rest-api)
- [Twenty GitHub](https://github.com/twentyhq/twenty)
