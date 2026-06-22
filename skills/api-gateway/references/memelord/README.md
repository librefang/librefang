# Memelord Routing Reference

**App name:** `memelord`
**Base URL proxied:** `www.memelord.com`

## API Path Pattern

```
/memelord/api/v1/{endpoint}
```

## Common Endpoints

### Generate Meme
```bash
POST /memelord/api/v1/ai-meme
Content-Type: application/json

{
  "prompt": "when the code finally compiles",
  "count": 3,
  "category": "trending",
  "include_nsfw": false
}
```

### Edit Meme
```bash
POST /memelord/api/v1/ai-meme/edit
Content-Type: application/json

{
  "instruction": "make it about debugging instead",
  "template_id": "success-kid-001",
  "template_data": {
    "top_text": "When the code compiles",
    "bottom_text": "On the first try"
  }
}
```

### Generate Video Meme
```bash
POST /memelord/api/v1/ai-video-meme
Content-Type: application/json

{
  "prompt": "explaining my code to a rubber duck",
  "count": 2,
  "webhookUrl": "https://your-server.com/webhook"
}
```

### Edit Video Meme
```bash
POST /memelord/api/v1/ai-video-meme/edit
Content-Type: application/json

{
  "instruction": "make it more dramatic",
  "template_id": "abc-123",
  "caption": "When the tests pass locally"
}
```

### Check Video Render Status
```bash
GET /memelord/api/video/render/remote?jobId={job_id}
```

## Notes

- Meme generation costs 1 credit per request
- Video meme generation costs 5 credits per request (multiplied by count)
- Video generation is asynchronous - use webhooks or polling
- Download URLs expire (memes: check expiration field, videos: 7 days)
- NSFW content is included by default; set `include_nsfw: false` to filter

## Resources

- [Memelord API Documentation](https://www.memelord.com/docs)
