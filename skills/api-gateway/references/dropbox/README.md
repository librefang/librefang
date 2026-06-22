# Dropbox Routing Reference

**App name:** `dropbox`
**Base URLs proxied:**
- `api.dropboxapi.com` - Standard RPC endpoints (metadata, search, etc.)
- `content.dropboxapi.com` - Content endpoints (upload, download)

Maton automatically routes to the correct host based on the endpoint path.

## API Path Pattern

```
/dropbox/2/{endpoint}
```

**Important:** All Dropbox API v2 endpoints use HTTP POST. Most endpoints use JSON request bodies, but upload/download endpoints use binary content with parameters in the `Dropbox-API-Arg` header.

## Common Endpoints

### Users

#### Get Current Account
```bash
POST /dropbox/2/users/get_current_account
Content-Type: application/json

null
```

#### Get Space Usage
```bash
POST /dropbox/2/users/get_space_usage
Content-Type: application/json

null
```

### Files

#### List Folder
```bash
POST /dropbox/2/files/list_folder
Content-Type: application/json

{
  "path": ""
}
```

Use empty string `""` for root folder.

#### Continue Listing
```bash
POST /dropbox/2/files/list_folder/continue
Content-Type: application/json

{
  "cursor": "..."
}
```

#### Get Metadata
```bash
POST /dropbox/2/files/get_metadata
Content-Type: application/json

{
  "path": "/document.pdf"
}
```

#### Create Folder
```bash
POST /dropbox/2/files/create_folder_v2
Content-Type: application/json

{
  "path": "/New Folder",
  "autorename": false
}
```

#### Copy
```bash
POST /dropbox/2/files/copy_v2
Content-Type: application/json

{
  "from_path": "/source/file.pdf",
  "to_path": "/destination/file.pdf"
}
```

#### Move
```bash
POST /dropbox/2/files/move_v2
Content-Type: application/json

{
  "from_path": "/old/file.pdf",
  "to_path": "/new/file.pdf"
}
```

#### Delete
```bash
POST /dropbox/2/files/delete_v2
Content-Type: application/json

{
  "path": "/file-to-delete.pdf"
}
```

#### Get Temporary Link
```bash
POST /dropbox/2/files/get_temporary_link
Content-Type: application/json

{
  "path": "/document.pdf"
}
```

### Upload (Content Endpoints)

Content endpoints use `Content-Type: application/octet-stream` with parameters in the `Dropbox-API-Arg` header.

#### Upload File (up to 150 MB)
```bash
POST /dropbox/2/files/upload
Content-Type: application/octet-stream
Dropbox-API-Arg: {"path": "/test.txt", "mode": "add", "autorename": true}

<file contents>
```

#### Upload Session Start
```bash
POST /dropbox/2/files/upload_session/start
Content-Type: application/octet-stream
Dropbox-API-Arg: {"close": false}

<first chunk>
```

#### Upload Session Append
```bash
POST /dropbox/2/files/upload_session/append_v2
Content-Type: application/octet-stream
Dropbox-API-Arg: {"cursor": {"session_id": "...", "offset": 10000000}, "close": false}

<next chunk>
```

#### Upload Session Finish
```bash
POST /dropbox/2/files/upload_session/finish
Content-Type: application/octet-stream
Dropbox-API-Arg: {"cursor": {"session_id": "...", "offset": 50000000}, "commit": {"path": "/file.zip", "mode": "add"}}

<final chunk>
```

#### Finish Batch Upload Sessions
```bash
POST /dropbox/2/files/upload_session/finish_batch
Content-Type: application/json

{
  "entries": [
    {
      "cursor": {"session_id": "...", "offset": 50000000},
      "commit": {"path": "/file1.zip", "mode": "add"}
    }
  ]
}
```

#### Check Batch Status
```bash
POST /dropbox/2/files/upload_session/finish_batch/check
Content-Type: application/json

{
  "async_job_id": "dbjid:..."
}
```

### Download (Content Endpoints)

#### Download File
```bash
POST /dropbox/2/files/download
Dropbox-API-Arg: {"path": "/document.pdf"}
```

#### Download ZIP
```bash
POST /dropbox/2/files/download_zip
Dropbox-API-Arg: {"path": "/folder"}
```

#### Export
```bash
POST /dropbox/2/files/export
Dropbox-API-Arg: {"path": "/document.paper"}
```

#### Get Preview
```bash
POST /dropbox/2/files/get_preview
Dropbox-API-Arg: {"path": "/document.docx"}
```

#### Get Thumbnail
```bash
POST /dropbox/2/files/get_thumbnail_v2
Dropbox-API-Arg: {"resource": {".tag": "path", "path": "/photo.jpg"}, "format": "jpeg", "size": "w128h128"}
```

### Search

#### Search Files
```bash
POST /dropbox/2/files/search_v2
Content-Type: application/json

{
  "query": "document"
}
```

### Revisions

#### List Revisions
```bash
POST /dropbox/2/files/list_revisions
Content-Type: application/json

{
  "path": "/document.pdf"
}
```

### Tags

#### Get Tags
```bash
POST /dropbox/2/files/tags/get
Content-Type: application/json

{
  "paths": ["/document.pdf"]
}
```

#### Add Tag
```bash
POST /dropbox/2/files/tags/add
Content-Type: application/json

{
  "path": "/document.pdf",
  "tag_text": "important"
}
```

#### Remove Tag
```bash
POST /dropbox/2/files/tags/remove
Content-Type: application/json

{
  "path": "/document.pdf",
  "tag_text": "important"
}
```

## Pagination

Dropbox uses cursor-based pagination:

```bash
POST /dropbox/2/files/list_folder
# Response includes "cursor" and "has_more": true/false

POST /dropbox/2/files/list_folder/continue
# Use cursor from previous response
```

## Notes

- All endpoints use POST method
- Standard endpoints use JSON request bodies (`Content-Type: application/json`)
- Content endpoints (upload/download) use binary content (`Content-Type: application/octet-stream`) with params in `Dropbox-API-Arg` header
- Gateway automatically routes content endpoints to `content.dropboxapi.com`
- Use empty string `""` for root folder path
- Paths are case-insensitive but case-preserving
- Tag text must match pattern `[\w]+` (alphanumeric and underscores)
- Temporary links expire after 4 hours
- Max single upload: 150 MB (use upload sessions for up to 350 GB)

## Content Endpoints (routed to content.dropboxapi.com)

The following endpoints are automatically routed to `content.dropboxapi.com`:
- `/2/files/upload`
- `/2/files/upload_session/start`
- `/2/files/upload_session/append_v2`
- `/2/files/upload_session/finish`
- `/2/files/download`
- `/2/files/download_zip`
- `/2/files/export`
- `/2/files/get_preview`
- `/2/files/get_thumbnail`
- `/2/files/get_thumbnail_v2`
- `/2/paper/docs/download`

## Resources

- [Dropbox HTTP API Overview](https://www.dropbox.com/developers/documentation/http/overview)
- [Dropbox API Explorer](https://dropbox.github.io/dropbox-api-v2-explorer/)
- [DBX File Access Guide](https://developers.dropbox.com/dbx-file-access-guide)
