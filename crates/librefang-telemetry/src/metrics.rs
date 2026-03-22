//! HTTP metrics utilities for LibreFang telemetry.

use dashmap::DashMap;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

lazy_static::lazy_static! {
    static ref HTTP_REQUEST_COUNTS: DashMap<String, Arc<AtomicU64>> = DashMap::new();
    static ref HTTP_REQUEST_LATENCIES: DashMap<String, Arc<RwLock<Vec<u64>>>> = DashMap::new();
}

pub fn normalize_path(path: &str) -> String {
    let segments: Vec<&str> = path.split('/').collect();
    let mut normalized = Vec::new();
    let mut i = 0;

    while i < segments.len() {
        let seg = segments[i];

        if seg.is_empty() {
            normalized.push(seg);
            i += 1;
            continue;
        }

        if seg == "api" || seg == "v1" || seg == "v2" || seg == "a2a" {
            normalized.push(seg);
            i += 1;
            continue;
        }

        if i + 1 < segments.len() {
            let next_seg = segments[i + 1];
            if is_dynamic_segment(next_seg) {
                normalized.push("{id}");
                i += 2;
                continue;
            }
        }

        normalized.push(seg);
        i += 1;
    }

    normalized.join("/")
}

fn is_dynamic_segment(s: &str) -> bool {
    if s.len() < 8 || s.len() > 64 {
        return false;
    }
    if s.contains('-') {
        return true;
    }
    s.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn record_http_request(path: &str, method: &str, status: u16, duration: Duration) {
    let key = format!("{}:{}:{}", method, normalize_path(path), status);

    let counters = HTTP_REQUEST_COUNTS
        .entry(key.clone())
        .or_insert_with(|| Arc::new(AtomicU64::new(0)));
    counters.fetch_add(1, Ordering::Relaxed);

    let latencies = HTTP_REQUEST_LATENCIES
        .entry(key)
        .or_insert_with(|| Arc::new(RwLock::new(Vec::with_capacity(1000))));
    let mut lat = latencies.write();
    let ms = duration.as_millis() as u64;
    if lat.len() < 1000 {
        lat.push(ms);
    }
}

pub fn get_http_metrics_summary() -> String {
    let mut output = String::new();

    output.push_str(
        "# HELP librefang_http_requests_total Total HTTP requests by method, path, and status.\n",
    );
    output.push_str("# TYPE librefang_http_requests_total counter\n");

    for entry in HTTP_REQUEST_COUNTS.iter() {
        let parts: Vec<&str> = entry.key().split(':').collect();
        if parts.len() == 3 {
            let method = parts[0];
            let path = parts[1];
            let status = parts[2];
            let count = entry.value().load(Ordering::Relaxed);
            output.push_str(&format!(
                "librefang_http_requests_total{{method=\"{}\",path=\"{}\",status=\"{}\"}} {}\n",
                method, path, status, count
            ));
        }
    }

    output.push('\n');
    output.push_str(
        "# HELP librefang_http_request_duration_ms HTTP request duration in milliseconds.\n",
    );
    output.push_str("# TYPE librefang_http_request_duration_ms summary\n");

    for entry in HTTP_REQUEST_LATENCIES.iter() {
        let parts: Vec<&str> = entry.key().split(':').collect();
        if parts.len() == 3 {
            let method = parts[0];
            let path = parts[1];
            let latencies = entry.value().read();
            if !latencies.is_empty() {
                let mut sorted = latencies.clone();
                sorted.sort();
                let len = sorted.len();
                let sum: u64 = sorted.iter().sum();
                let p50 = sorted[len / 2];
                let p90 = sorted[(len as f64 * 0.9) as usize].min(sorted[len - 1]);
                let p99 = sorted[(len as f64 * 0.99) as usize].min(sorted[len - 1]);

                output.push_str(&format!(
                    "librefang_http_request_duration_ms_sum{{method=\"{}\",path=\"{}\"}} {}\n",
                    method, path, sum
                ));
                output.push_str(&format!(
                    "librefang_http_request_duration_ms_count{{method=\"{}\",path=\"{}\"}} {}\n",
                    method, path, len
                ));
                output.push_str(&format!(
                    "librefang_http_request_duration_ms_p50{{method=\"{}\",path=\"{}\"}} {}\n",
                    method, path, p50
                ));
                output.push_str(&format!(
                    "librefang_http_request_duration_ms_p90{{method=\"{}\",path=\"{}\"}} {}\n",
                    method, path, p90
                ));
                output.push_str(&format!(
                    "librefang_http_request_duration_ms_p99{{method=\"{}\",path=\"{}\"}} {}\n",
                    method, path, p99
                ));
            }
        }
    }

    output
}
