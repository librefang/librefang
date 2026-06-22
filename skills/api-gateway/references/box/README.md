# Box Routing Reference

**App name:** `box`
**Base URLs proxied:**
- `api.box.com` - Standard API endpoints (metadata, folders, search, etc.)
- `upload.box.com` - Upload endpoints (file upload, chunked upload sessions)

Maton automatically routes to the correct host based on the endpoint path.

## API Path Pattern

```
/box/2.0/{resource}
/box/api/2.0/{resource}  # Upload endpoints
```

## Common Endpoints

### Get Current User
```bash
GET /box/2.0/users/me
```

### Get User
```bash
GET /box/2.0/users/{user_id}
```

### Get Folder
```bash
GET /box/2.0/folders/{folder_id}
```

Root folder ID is `0`.

### List Folder Items
```bash
GET /box/2.0/folders/{folder_id}/items
GET /box/2.0/folders/{folder_id}/items?limit=100&offset=0
```

### Create Folder
```bash
POST /box/2.0/folders
Content-Type: application/json

{
  "name": "New Folder",
  "parent": {"id": "0"}
}
```

### Update Folder
```bash
PUT /box/2.0/folders/{folder_id}
Content-Type: application/json

{
  "name": "Updated Name",
  "description": "Description"
}
```

### Copy Folder
```bash
POST /box/2.0/folders/{folder_id}/copy
Content-Type: application/json

{
  "name": "Copied Folder",
  "parent": {"id": "0"}
}
```

### Delete Folder
```bash
DELETE /box/2.0/folders/{folder_id}
DELETE /box/2.0/folders/{folder_id}?recursive=true
```

### Get File
```bash
GET /box/2.0/files/{file_id}
```

### Download File
```bash
GET /box/2.0/files/{file_id}/content
```

### Update File
```bash
PUT /box/2.0/files/{file_id}
```

### Copy File
```bash
POST /box/2.0/files/{file_id}/copy
```

### Delete File
```bash
DELETE /box/2.0/files/{file_id}
```

### Upload File (up to 50 MB)
```bash
POST /box/api/2.0/files/content
Content-Type: multipart/form-data

attributes={"name":"file.txt","parent":{"id":"0"}}
file=<binary data>
```

### Upload New File Version
```bash
POST /box/api/2.0/files/{file_id}/content
Content-Type: multipart/form-data

attributes={"name":"file.txt"}
file=<binary data>
```

### Chunked Upload (Large Files)

#### Create Upload Session
```bash
POST /box/api/2.0/files/upload_sessions
Content-Type: application/json

{
  "folder_id": "0",
  "file_size": 104857600,
  "file_name": "large_file.zip"
}
```

#### Create Upload Session for New Version
```bash
POST /box/api/2.0/files/{file_id}/upload_sessions
Content-Type: application/json

{
  "file_size": 104857600,
  "file_name": "large_file.zip"
}
```

#### Upload Part
```bash
PUT /box/api/2.0/files/upload_sessions/{session_id}
Content-Type: application/octet-stream
Content-Range: bytes 0-8388607/104857600
Digest: sha=<base64-encoded SHA-1>

<part data>
```

#### List Parts
```bash
GET /box/api/2.0/files/upload_sessions/{session_id}/parts
```

#### Commit Upload Session
```bash
POST /box/api/2.0/files/upload_sessions/{session_id}/commit
Content-Type: application/json
Digest: sha=<base64-encoded SHA-1 of entire file>

{
  "parts": [
    {"part_id": "...", "offset": 0, "size": 8388608}
  ]
}
```

#### Abort Upload Session
```bash
DELETE /box/api/2.0/files/upload_sessions/{session_id}
```

### Create Shared Link
```bash
PUT /box/2.0/folders/{folder_id}
Content-Type: application/json

{
  "shared_link": {"access": "open"}
}
```

### List Collaborations
```bash
GET /box/2.0/folders/{folder_id}/collaborations
```

### Create Collaboration
```bash
POST /box/2.0/collaborations
Content-Type: application/json

{
  "item": {"type": "folder", "id": "123"},
  "accessible_by": {"type": "user", "login": "user@example.com"},
  "role": "editor"
}
```

### Search
```bash
GET /box/2.0/search?query=keyword
```

### Events
```bash
GET /box/2.0/events
```

### Trash
```bash
GET /box/2.0/folders/trash/items
DELETE /box/2.0/files/{file_id}/trash
DELETE /box/2.0/folders/{folder_id}/trash
```

### Collections
```bash
GET /box/2.0/collections
GET /box/2.0/collections/{collection_id}/items
```

### Recent Items
```bash
GET /box/2.0/recent_items
```

### Webhooks
```bash
GET /box/2.0/webhooks
POST /box/2.0/webhooks
DELETE /box/2.0/webhooks/{webhook_id}
```

## Pagination

Offset-based pagination:
```bash
GET /box/2.0/folders/0/items?limit=100&offset=0
```

Response:
```json
{
  "total_count": 250,
  "entries": [...],
  "offset": 0,
  "limit": 100
}
```

## Notes

- Root folder ID is `0`
- Gateway automatically routes upload endpoints to `upload.box.com`
- Direct upload supports files up to 50 MB
- Use chunked upload sessions for files up to 50 GB
- Chunked uploads require SHA-1 digest headers
- Delete operations return 204 No Content
- Some operations require enterprise admin permissions
- Use `fields` parameter to select specific fields

## Upload Endpoints (routed to upload.box.com)

The following endpoints are automatically routed to `upload.box.com`:
- `/api/2.0/files/content` - Direct file upload
- `/api/2.0/files/{file_id}/content` - Upload new file version
- `/api/2.0/files/upload_sessions` - Create upload session
- `/api/2.0/files/upload_sessions/*` - All upload session operations
- `/api/2.0/files/{file_id}/upload_sessions` - Create version upload session

## Resources

- [Box API Reference](https://developer.box.com/reference)
- [Box Developer Documentation](https://developer.box.com/guides)
