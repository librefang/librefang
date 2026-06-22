# Apify Routing Reference

**App name:** `apify`
**Base URL proxied:** `api.apify.com`

## API Path Pattern

```
/apify/v2/{resource}
```

## Common Endpoints

### Users

#### Get Current User
```bash
GET /apify/v2/users/me
```

### Actors

#### List Actors
```bash
GET /apify/v2/acts
GET /apify/v2/acts?my=true
```

#### Get Actor
```bash
GET /apify/v2/acts/{actorId}
```

#### Run Actor
```bash
POST /apify/v2/acts/{actorId}/runs
Content-Type: application/json

{
  "startUrls": [{"url": "https://example.com"}],
  "maxItems": 100
}
```

### Actor Runs

#### List Runs
```bash
GET /apify/v2/actor-runs
GET /apify/v2/actor-runs?status=SUCCEEDED
```

#### Get Run
```bash
GET /apify/v2/actor-runs/{runId}
```

#### Abort Run
```bash
POST /apify/v2/actor-runs/{runId}/abort
```

### Actor Tasks

#### List Tasks
```bash
GET /apify/v2/actor-tasks
```

#### Get Task
```bash
GET /apify/v2/actor-tasks/{actorTaskId}
```

#### Run Task
```bash
POST /apify/v2/actor-tasks/{actorTaskId}/runs
```

### Datasets

#### List Datasets
```bash
GET /apify/v2/datasets
```

#### Get Dataset
```bash
GET /apify/v2/datasets/{datasetId}
```

#### Get Dataset Items
```bash
GET /apify/v2/datasets/{datasetId}/items
GET /apify/v2/datasets/{datasetId}/items?format=json&clean=true
```

#### Put Items
```bash
POST /apify/v2/datasets/{datasetId}/items
Content-Type: application/json

[{"field1": "value1"}, {"field2": "value2"}]
```

### Key-Value Stores

#### List Stores
```bash
GET /apify/v2/key-value-stores
```

#### Get Store
```bash
GET /apify/v2/key-value-stores/{storeId}
```

#### Get Record
```bash
GET /apify/v2/key-value-stores/{storeId}/records/{key}
```

#### Set Record
```bash
PUT /apify/v2/key-value-stores/{storeId}/records/{key}
Content-Type: application/json

{"data": "value"}
```

### Request Queues

#### List Queues
```bash
GET /apify/v2/request-queues
```

#### Get Queue
```bash
GET /apify/v2/request-queues/{queueId}
```

#### Add Request
```bash
POST /apify/v2/request-queues/{queueId}/requests
Content-Type: application/json

{
  "url": "https://example.com",
  "uniqueKey": "unique-key"
}
```

### Schedules

#### List Schedules
```bash
GET /apify/v2/schedules
```

#### Get Schedule
```bash
GET /apify/v2/schedules/{scheduleId}
```

#### Create Schedule
```bash
POST /apify/v2/schedules
Content-Type: application/json

{
  "name": "My Schedule",
  "cronExpression": "0 0 * * *",
  "actorId": "actor-id"
}
```

### Webhooks

#### List Webhooks
```bash
GET /apify/v2/webhooks
```

#### Get Webhook
```bash
GET /apify/v2/webhooks/{webhookId}
```

#### Create Webhook
```bash
POST /apify/v2/webhooks
Content-Type: application/json

{
  "eventTypes": ["ACTOR.RUN.SUCCEEDED"],
  "requestUrl": "https://example.com/webhook"
}
```

## Pagination

Offset-based pagination:

```bash
GET /apify/v2/acts?offset=0&limit=100
```

Response includes:
```json
{
  "data": {
    "total": 150,
    "offset": 0,
    "limit": 100,
    "count": 100,
    "items": [...]
  }
}
```

## Query Parameters

Common parameters:
- `offset` - Number of items to skip (default: 0)
- `limit` - Max items to return (default: varies, max: 1000)
- `desc` - Sort descending by creation date (boolean)

For dataset items:
- `format` - Response format (json, csv, xlsx, xml, rss)
- `clean` - Remove empty fields (boolean)
- `fields` - Comma-separated field names to include

## Notes

- All endpoints use the `/v2/` prefix
- Actor IDs can be `username/actor-name` or unique IDs
- Timestamps are ISO 8601 format
- Default response format is JSON
- Rate limits apply per account

## Resources

- [Apify API Reference](https://docs.apify.com/api/v2)
- [Apify Platform Documentation](https://docs.apify.com/platform)
