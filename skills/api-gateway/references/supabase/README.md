# Supabase Routing Reference

**App name:** `supabase`
**Base URL proxied:** `{project_ref}.supabase.co`

## API Path Pattern

```
/supabase/{service}/{native-api-path}
```

Services:
- `rest/v1` - PostgREST API (database tables)
- `auth/v1` - GoTrue authentication API
- `storage/v1` - Storage API

## Common Endpoints

### Database (PostgREST)

#### Get OpenAPI Schema
```bash
GET /supabase/rest/v1/
```

#### List Records
```bash
GET /supabase/rest/v1/{table_name}?select=*&limit=10
```

#### Get Single Record
```bash
GET /supabase/rest/v1/{table_name}?id=eq.{id}
```

#### Insert Record
```bash
POST /supabase/rest/v1/{table_name}
Content-Type: application/json
Prefer: return=representation

{"name": "value"}
```

#### Update Record
```bash
PATCH /supabase/rest/v1/{table_name}?id=eq.{id}
Content-Type: application/json
Prefer: return=representation

{"name": "new_value"}
```

#### Delete Record
```bash
DELETE /supabase/rest/v1/{table_name}?id=eq.{id}
```

### Auth (GoTrue)

#### Get Health
```bash
GET /supabase/auth/v1/health
```

#### Get Settings
```bash
GET /supabase/auth/v1/settings
```

#### List Users (Admin)
```bash
GET /supabase/auth/v1/admin/users
```

#### Get User (Admin)
```bash
GET /supabase/auth/v1/admin/users/{user_id}
```

#### Create User (Admin)
```bash
POST /supabase/auth/v1/admin/users
Content-Type: application/json

{
  "email": "user@example.com",
  "password": "password123",
  "email_confirm": true
}
```

#### Delete User (Admin)
```bash
DELETE /supabase/auth/v1/admin/users/{user_id}
```

### Storage

#### List Buckets
```bash
GET /supabase/storage/v1/bucket
```

#### Get Bucket
```bash
GET /supabase/storage/v1/bucket/{bucket_id}
```

#### Create Bucket
```bash
POST /supabase/storage/v1/bucket
Content-Type: application/json

{
  "id": "my-bucket",
  "name": "my-bucket",
  "public": false
}
```

#### Delete Bucket
```bash
DELETE /supabase/storage/v1/bucket/{bucket_id}
```

#### List Objects
```bash
POST /supabase/storage/v1/object/list/{bucket_id}
Content-Type: application/json

{"prefix": "", "limit": 100}
```

#### Upload Object
```bash
POST /supabase/storage/v1/object/{bucket_id}/{path}
Content-Type: {mime_type}

{binary_data}
```

#### Download Object
```bash
GET /supabase/storage/v1/object/{bucket_id}/{path}
```

#### Delete Object
```bash
DELETE /supabase/storage/v1/object/{bucket_id}/{path}
```

## Pagination

### PostgREST
```bash
GET /supabase/rest/v1/{table}?limit=10&offset=20
```

Or use Range header:
```
Range: 0-9
```

### Auth Users
```bash
GET /supabase/auth/v1/admin/users?page=1&per_page=50
```

## PostgREST Filter Operators

| Operator | Meaning | Example |
|----------|---------|---------|
| `eq` | Equals | `?status=eq.active` |
| `neq` | Not equals | `?status=neq.deleted` |
| `gt` | Greater than | `?age=gt.18` |
| `lt` | Less than | `?age=lt.65` |
| `like` | Pattern match | `?name=like.*john*` |
| `in` | In list | `?status=in.(active,pending)` |
| `is` | Is null | `?deleted_at=is.null` |

## Notes

- Connection routes to a specific Supabase project
- PostgREST endpoints auto-generate from database schema
- Use `Prefer: return=representation` to get created/updated records
- Auth admin endpoints require service role permissions
- Bucket names must be unique within project

## Resources

- [Supabase REST API Guide](https://supabase.com/docs/guides/api)
- [PostgREST Documentation](https://postgrest.org/en/stable/)
- [Supabase Auth API](https://supabase.com/docs/reference/javascript/auth-api)
- [Supabase Storage API](https://supabase.com/docs/reference/javascript/storage-api)
