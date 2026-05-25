//! System info tools: location lookup and clock readout.
//!
//! `tool_location_get` migrated from `Result<String, String>` to
//! `Result<String, ToolError>` (#3576). All failures are external HTTP / parse
//! failures, mapped to `ToolError::Upstream`. The typed `reqwest::Error`s
//! (build / send / json) keep their original prefixed message AND preserve the
//! error on the `source()` chain (consistent with how the sibling slices wrap
//! typed errors); the two non-`Error` API-level failures use `upstream_msg`.
//! `Upstream`'s Display is the bare message, so the exact strings are
//! preserved. `tool_system_time` is infallible (returns `String`), unchanged.

use super::error::{ToolError, ToolResult};

/// Look up approximate location via ip-api.com.
pub(super) async fn tool_location_get() -> ToolResult {
    let client = crate::http_client::proxied_client_builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| ToolError::Upstream {
            message: format!("Failed to create HTTP client: {e}"),
            source: Some(Box::new(e)),
        })?;

    // Use ip-api.com (free, no API key, JSON response)
    let resp = client
        .get("https://ip-api.com/json/?fields=status,message,country,regionName,city,zip,lat,lon,timezone,isp,query")
        .header("User-Agent", "LibreFang/0.1")
        .send()
        .await
        .map_err(|e| ToolError::Upstream {
            message: format!("Location request failed: {e}"),
            source: Some(Box::new(e)),
        })?;

    if !resp.status().is_success() {
        return Err(ToolError::upstream_msg(format!(
            "Location API returned {}",
            resp.status()
        )));
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| ToolError::Upstream {
        message: format!("Failed to parse location response: {e}"),
        source: Some(Box::new(e)),
    })?;

    if body["status"].as_str() != Some("success") {
        let msg = body["message"].as_str().unwrap_or("Unknown error");
        return Err(ToolError::upstream_msg(format!(
            "Location lookup failed: {msg}"
        )));
    }

    let result = serde_json::json!({
        "lat": body["lat"],
        "lon": body["lon"],
        "city": body["city"],
        "region": body["regionName"],
        "country": body["country"],
        "zip": body["zip"],
        "timezone": body["timezone"],
        "isp": body["isp"],
        "ip": body["query"],
    });

    Ok(serde_json::to_string_pretty(&result)?)
}

/// Return current date, time, timezone, and Unix epoch.
pub(super) fn tool_system_time() -> String {
    let now_utc = chrono::Utc::now();
    let now_local = chrono::Local::now();
    let result = serde_json::json!({
        "utc": now_utc.to_rfc3339(),
        "local": now_local.to_rfc3339(),
        "unix_epoch": now_utc.timestamp(),
        "timezone": now_local.format("%Z").to_string(),
        "utc_offset": now_local.format("%:z").to_string(),
        "date": now_local.format("%Y-%m-%d").to_string(),
        "time": now_local.format("%H:%M:%S").to_string(),
        "day_of_week": now_local.format("%A").to_string(),
    });
    serde_json::to_string_pretty(&result).unwrap_or_else(|_| now_utc.to_rfc3339())
}
