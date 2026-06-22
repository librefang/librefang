# Vercel Routing Reference

**App name:** `vercel`
**Base URL proxied:** `api.vercel.com`

## API Path Pattern

```
/vercel/{api-version}/{resource}
```

Note: API versions vary by endpoint (v2, v5, v6, v9, v10, v13, etc.)

## Common Endpoints

### User

#### Get Current User
```bash
GET /vercel/v2/user
```

### Teams

#### List Teams
```bash
GET /vercel/v2/teams
```

### Projects

#### List Projects
```bash
GET /vercel/v9/projects?limit=20
```

#### Get Project
```bash
GET /vercel/v9/projects/{projectId}
```

#### Create Project
```bash
POST /vercel/v9/projects
Content-Type: application/json

{
  "name": "my-project",
  "framework": "nextjs",
  "gitRepository": {
    "type": "github",
    "repo": "username/repo"
  }
}
```

#### Update Project
```bash
PATCH /vercel/v9/projects/{projectId}
Content-Type: application/json

{
  "name": "updated-name"
}
```

#### Delete Project
```bash
DELETE /vercel/v9/projects/{projectId}
```

### Deployments

#### List Deployments
```bash
GET /vercel/v6/deployments?limit=20
GET /vercel/v6/deployments?projectId={projectId}&limit=20
```

#### Get Deployment
```bash
GET /vercel/v13/deployments/{deploymentId}
```

#### Get Deployment Build Logs
```bash
GET /vercel/v3/deployments/{deploymentId}/events
```

#### Cancel Deployment
```bash
PATCH /vercel/v12/deployments/{deploymentId}/cancel
```

### Environment Variables

#### List Environment Variables
```bash
GET /vercel/v10/projects/{projectId}/env
```

#### Create Environment Variable
```bash
POST /vercel/v10/projects/{projectId}/env
Content-Type: application/json

{
  "key": "API_KEY",
  "value": "secret-value",
  "type": "encrypted",
  "target": ["production", "preview"]
}
```

#### Update Environment Variable
```bash
PATCH /vercel/v10/projects/{projectId}/env/{envId}
Content-Type: application/json

{
  "value": "new-value"
}
```

#### Delete Environment Variable
```bash
DELETE /vercel/v10/projects/{projectId}/env/{envId}
```

### Domains

#### List Domains
```bash
GET /vercel/v5/domains
```

#### Get Domain
```bash
GET /vercel/v5/domains/{domain}
```

#### Add Domain
```bash
POST /vercel/v5/domains
Content-Type: application/json

{
  "name": "example.com"
}
```

#### Remove Domain
```bash
DELETE /vercel/v6/domains/{domain}
```

### Remote Caching (Artifacts)

#### Get Artifacts Status
```bash
GET /vercel/v8/artifacts/status
```

## Pagination

Cursor-based pagination:

```bash
GET /vercel/v9/projects?limit=20&until={next}
```

Parameters:
- `limit` - Results per page (max varies by endpoint, typically 100)
- `until` - Cursor for next page
- `since` - Cursor for previous page

Response pagination:
```json
{
  "pagination": {
    "count": 20,
    "next": 1733304037737,
    "prev": 1759739951209
  }
}
```

## Notes

- API versions vary by endpoint (v2, v5, v6, v9, v10, v13)
- Timestamps are in milliseconds since Unix epoch
- Project IDs start with `prj_`
- Deployment IDs start with `dpl_`
- Team IDs start with `team_`
- Deployment states: `BUILDING`, `READY`, `ERROR`, `CANCELED`, `QUEUED`
- Environment variable types: `plain`, `encrypted`, `secret`, `sensitive`
- Environment targets: `production`, `preview`, `development`

## Resources

- [Vercel REST API Documentation](https://vercel.com/docs/rest-api)
- [Vercel API Reference](https://vercel.com/docs/rest-api/endpoints)
