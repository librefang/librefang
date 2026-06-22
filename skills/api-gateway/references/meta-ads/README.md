# Meta Ads Routing Reference

**App name:** `meta-ads`
**Base URL proxied:** `graph.facebook.com`

## API Path Pattern

```
/meta-ads/v25.0/{resource}
```

## Common Endpoints

### User & Ad Accounts

#### Get Current User
```bash
GET /meta-ads/v25.0/me?fields=id,name,email
```

#### List Ad Accounts
```bash
GET /meta-ads/v25.0/me/adaccounts?fields=id,name,account_status,currency,amount_spent
```

#### Get Ad Account Details
```bash
GET /meta-ads/v25.0/act_{ad_account_id}?fields=id,name,account_status,currency,spend_cap,amount_spent,balance
```

### Campaigns

#### List Campaigns
```bash
GET /meta-ads/v25.0/act_{ad_account_id}/campaigns?fields=id,name,status,objective,daily_budget,lifetime_budget
```

#### Get Campaign
```bash
GET /meta-ads/v25.0/{campaign_id}?fields=id,name,status,objective
```

#### Create Campaign
```bash
POST /meta-ads/v25.0/act_{ad_account_id}/campaigns
Content-Type: application/json

{
  "name": "My Campaign",
  "objective": "OUTCOME_TRAFFIC",
  "status": "PAUSED",
  "special_ad_categories": []
}
```

#### Update Campaign
```bash
POST /meta-ads/v25.0/{campaign_id}
Content-Type: application/json

{
  "name": "Updated Name",
  "status": "ACTIVE"
}
```

#### Delete Campaign
```bash
DELETE /meta-ads/v25.0/{campaign_id}
```

### Ad Sets

#### List Ad Sets
```bash
GET /meta-ads/v25.0/act_{ad_account_id}/adsets?fields=id,name,status,campaign_id,daily_budget,optimization_goal
```

#### Create Ad Set
```bash
POST /meta-ads/v25.0/act_{ad_account_id}/adsets
Content-Type: application/json

{
  "name": "My Ad Set",
  "campaign_id": "{campaign_id}",
  "daily_budget": 1000,
  "billing_event": "IMPRESSIONS",
  "optimization_goal": "LINK_CLICKS",
  "status": "PAUSED",
  "targeting": {
    "geo_locations": {"countries": ["US"]}
  }
}
```

### Ads

#### List Ads
```bash
GET /meta-ads/v25.0/act_{ad_account_id}/ads?fields=id,name,status,adset_id,creative,effective_status
```

#### Create Ad
```bash
POST /meta-ads/v25.0/act_{ad_account_id}/ads
Content-Type: application/json

{
  "name": "My Ad",
  "adset_id": "{adset_id}",
  "creative": {"creative_id": "{creative_id}"},
  "status": "PAUSED"
}
```

### Ad Creatives

#### List Ad Creatives
```bash
GET /meta-ads/v25.0/act_{ad_account_id}/adcreatives?fields=id,name,object_story_spec,thumbnail_url
```

#### Create Ad Creative
```bash
POST /meta-ads/v25.0/act_{ad_account_id}/adcreatives
Content-Type: application/json

{
  "name": "My Creative",
  "object_story_spec": {
    "page_id": "{page_id}",
    "link_data": {
      "link": "https://example.com",
      "message": "Check out our website!"
    }
  }
}
```

### Ad Images

#### List Ad Images
```bash
GET /meta-ads/v25.0/act_{ad_account_id}/adimages?fields=hash,name,url
```

### Insights (Analytics)

#### Get Account Insights
```bash
GET /meta-ads/v25.0/act_{ad_account_id}/insights?fields=impressions,clicks,spend,reach,cpc,cpm,ctr&date_preset=last_7d
```

#### Get Campaign Insights
```bash
GET /meta-ads/v25.0/{campaign_id}/insights?fields=impressions,clicks,spend,reach,actions&date_preset=last_30d
```

#### Get Insights with Breakdowns
```bash
GET /meta-ads/v25.0/act_{ad_account_id}/insights?fields=impressions,clicks,spend&breakdowns=age,gender&date_preset=last_7d
```

#### Get Insights with Custom Date Range
```bash
GET /meta-ads/v25.0/act_{ad_account_id}/insights?fields=impressions,clicks,spend&time_range={"since":"2026-01-01","until":"2026-01-31"}
```

### Custom Audiences

#### List Custom Audiences
```bash
GET /meta-ads/v25.0/act_{ad_account_id}/customaudiences?fields=id,name,subtype,approximate_count
```

## Notes

- Ad Account IDs are prefixed with `act_` (e.g., `act_123456789`)
- Campaign objectives: `OUTCOME_AWARENESS`, `OUTCOME_ENGAGEMENT`, `OUTCOME_LEADS`, `OUTCOME_SALES`, `OUTCOME_TRAFFIC`, `OUTCOME_APP_PROMOTION`
- Status values: `ACTIVE`, `PAUSED`, `DELETED`, `ARCHIVED`
- Budget values are in account currency minor units (cents for USD)
- `special_ad_categories` required for campaigns: `[]`, `HOUSING`, `EMPLOYMENT`, `CREDIT`, `ISSUES_ELECTIONS_POLITICS`
- Date presets: `today`, `yesterday`, `this_month`, `last_month`, `last_7d`, `last_14d`, `last_28d`, `last_30d`, `last_90d`
- Breakdowns: `age`, `gender`, `country`, `region`, `dma`, `impression_device`, `platform_position`, `publisher_platform`
- Pagination uses cursor-based navigation with `after` parameter
- API version updates frequently - check Meta docs for latest

## Resources

- [Marketing API Overview](https://developers.facebook.com/docs/marketing-api/overview)
- [Ad Account Reference](https://developers.facebook.com/docs/marketing-api/reference/ad-account)
- [Campaign Reference](https://developers.facebook.com/docs/marketing-api/reference/ad-campaign-group)
- [Ad Set Reference](https://developers.facebook.com/docs/marketing-api/reference/ad-campaign)
- [Insights API](https://developers.facebook.com/docs/marketing-api/insights)
