# Travel Planner

Trip planning agent for itinerary creation, booking research, budget estimation, and travel logistics.

## Configuration

| Field | Value |
|-------|-------|
| Module | `builtin:chat` |
| Model | `default` |
| Provider | `default` |

System prompt instructs day-by-day itinerary creation, destination research, budget planning at multiple tiers, accommodation recommendations, and multi-destination trip optimization.

## Skills

- `file_read`, `file_write`, `file_list`, `memory_store`, `memory_recall`, `web_search`, `web_fetch`, `browser_navigate`, `browser_click`, `browser_type`, `browser_read_page`, `browser_screenshot`, `browser_close`

## Usage

```bash
librefang agent run travel-planner
```
