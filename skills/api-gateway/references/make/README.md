# Make Routing Reference

**App name:** `make`
**Base URL proxied:** `{zone}.make.com` (e.g., `us1.make.com`, `eu1.make.com`)

## API Path Pattern

```
/make/api/v2/{resource}
```

## Common Endpoints

### Users

#### Get Current User
```bash
GET /make/api/v2/users/me
```

#### List Users
```bash
GET /make/api/v2/users?organizationId={organizationId}
GET /make/api/v2/users?teamId={teamId}
```

### Organizations

#### List Organizations
```bash
GET /make/api/v2/organizations
```

#### Get Organization
```bash
GET /make/api/v2/organizations/{organizationId}
```

#### Create Organization
```bash
POST /make/api/v2/organizations
Content-Type: application/json

{
  "name": "Organization Name",
  "regionId": 2,
  "timezoneId": 301,
  "countryId": 202
}
```

#### Get Organization Usage
```bash
GET /make/api/v2/organizations/{organizationId}/usage
```

### Teams

#### List Teams
```bash
GET /make/api/v2/teams?organizationId={organizationId}
```

#### Get Team
```bash
GET /make/api/v2/teams/{teamId}
```

#### Create Team
```bash
POST /make/api/v2/teams
Content-Type: application/json

{
  "name": "Team Name",
  "organizationId": 123456
}
```

### Scenarios

#### List Scenarios
```bash
GET /make/api/v2/scenarios?organizationId={organizationId}
GET /make/api/v2/scenarios?teamId={teamId}
```

#### Get Scenario
```bash
GET /make/api/v2/scenarios/{scenarioId}
```

#### Create Scenario
```bash
POST /make/api/v2/scenarios
Content-Type: application/json

{
  "teamId": 123456,
  "name": "Scenario Name",
  "blueprint": "{...}"
}
```

#### Start/Stop Scenario
```bash
POST /make/api/v2/scenarios/{scenarioId}/start
POST /make/api/v2/scenarios/{scenarioId}/stop
```

#### Run Scenario
```bash
POST /make/api/v2/scenarios/{scenarioId}/run
```

#### Get Scenario Logs
```bash
GET /make/api/v2/scenarios/{scenarioId}/logs
```

### Connections (App Connections)

#### List Connections
```bash
GET /make/api/v2/connections?teamId={teamId}
```

#### Test Connection
```bash
POST /make/api/v2/connections/{connectionId}/test
```

### Data Stores

#### List Data Stores
```bash
GET /make/api/v2/data-stores?teamId={teamId}
```

#### Get Data Store
```bash
GET /make/api/v2/data-stores/{dataStoreId}
```

### Hooks (Webhooks)

#### List Hooks
```bash
GET /make/api/v2/hooks?teamId={teamId}
```

#### Enable/Disable Hook
```bash
POST /make/api/v2/hooks/{hookId}/enable
POST /make/api/v2/hooks/{hookId}/disable
```

### Templates

#### List Templates
```bash
GET /make/api/v2/templates?teamId={teamId}
```

#### Get Template Blueprint
```bash
GET /make/api/v2/templates/{templateId}/blueprint
```

### Incomplete Executions (DLQs)

#### List Incomplete Executions
```bash
GET /make/api/v2/dlqs?scenarioId={scenarioId}
```

#### Retry Incomplete Execution
```bash
POST /make/api/v2/dlqs/{dlqId}/retry
```

## Pagination

Offset-based pagination using `pg` parameters:

```bash
GET /make/api/v2/scenarios?organizationId=123&pg[offset]=0&pg[limit]=50
```

Response includes:
```json
{
  "scenarios": [...],
  "pg": {
    "sortBy": "name",
    "limit": 500,
    "sortDir": "asc",
    "offset": 0
  }
}
```

## Notes

- Most list endpoints require either `organizationId` or `teamId`
- Make uses zone-specific URLs - gateway routes automatically based on connection
- Zone URLs: `us1.make.com`, `us2.make.com`, `eu1.make.com`, `eu2.make.com`
- All IDs are integers
- Timestamps use ISO 8601 format

## Resources

- [Make API Documentation](https://developers.make.com/api-documentation)
- [Make API Reference](https://developers.make.com/api-documentation/api-reference)
