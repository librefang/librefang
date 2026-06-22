# Facebook Page Routing Reference

**App name:** `facebook-page`
**Base URL proxied:** `graph.facebook.com`

## API Path Pattern

```
/facebook-page/v25.0/{resource}
```

## Page Access Token

Most page-specific endpoints require a **Page Access Token** passed as the `access_token` query parameter. The gateway injects a User Access Token, but endpoints like feed, insights, and comments need a Page Access Token.

**Two-step flow:**
1. Get the page access token: `GET /facebook-page/v25.0/me/accounts?fields=id,name,access_token`
2. Pass it as `access_token` query parameter in subsequent calls

Endpoints that work with User Access Token only (no `access_token` param needed):
- `GET /facebook-page/v25.0/me/accounts`
- `GET /facebook-page/v25.0/{page_id}`

## Common Endpoints

### List Pages
```bash
GET /facebook-page/v25.0/me/accounts?fields=id,name,category,fan_count,followers_count
```

### Get Page Details
```bash
GET /facebook-page/v25.0/{page_id}?fields=id,name,about,category,fan_count,followers_count,website,link
```

### Get Page Feed
```bash
GET /facebook-page/v25.0/{page_id}/feed?fields=id,message,created_time&limit=10&access_token={page_access_token}
```

### Get Published Posts
```bash
GET /facebook-page/v25.0/{page_id}/published_posts?fields=id,message,created_time&limit=10&access_token={page_access_token}
```

### Publish a Post
```bash
POST /facebook-page/v25.0/{page_id}/feed?access_token={page_access_token}
Content-Type: application/json

{
  "message": "Hello from my page!"
}
```

### Update a Post
```bash
POST /facebook-page/v25.0/{post_id}?access_token={page_access_token}
Content-Type: application/json

{
  "message": "Updated post content"
}
```

### Delete a Post
```bash
DELETE /facebook-page/v25.0/{post_id}?access_token={page_access_token}
```

### Get Comments on a Post
```bash
GET /facebook-page/v25.0/{post_id}/comments?fields=id,message,from,created_time&access_token={page_access_token}
```

### Post a Comment
```bash
POST /facebook-page/v25.0/{post_id}/comments?access_token={page_access_token}
Content-Type: application/json

{
  "message": "Thanks for your feedback!"
}
```

### Get Page Insights
```bash
GET /facebook-page/v25.0/{page_id}/insights?metric=page_views_total,page_posts_impressions,page_video_views&period=day&access_token={page_access_token}
```

### Get Page Insights with Date Range
```bash
GET /facebook-page/v25.0/{page_id}/insights?metric=page_views_total&period=day&since=2026-01-01&until=2026-01-31&access_token={page_access_token}
```

### Get Page Photos
```bash
GET /facebook-page/v25.0/{page_id}/photos?fields=id,name,created_time,images&limit=10&access_token={page_access_token}
```

### Get Page Videos
```bash
GET /facebook-page/v25.0/{page_id}/videos?fields=id,title,description,created_time&limit=10&access_token={page_access_token}
```

### Get Product Catalogs
```bash
GET /facebook-page/v25.0/{page_id}/product_catalogs?access_token={page_access_token}
```

#### Get Products in a Catalog
```bash
GET /facebook-page/v25.0/{catalog_id}/products?fields=id,name,price,image_url&access_token={page_access_token}
```

## Notes

- Post IDs follow the format `{page_id}_{post_id}`
- Uses cursor-based pagination with `before`/`after` cursors
- Maximum 100 results per page for feed endpoints
- Approximately 600 ranked, published posts per year are accessible
- Insight period values: `day`, `week`, `days_28`
- Deprecated metrics: use `page_views_total` instead of `page_impressions`, `page_posts_impressions` instead of `page_engaged_users`

## Resources

- [Facebook Graph API Overview](https://developers.facebook.com/docs/graph-api/overview)
- [Page API Reference](https://developers.facebook.com/docs/graph-api/reference/page/)
- [Pages API Getting Started](https://developers.facebook.com/docs/pages-api/getting-started)
