use crate::common::repo_root;
use clap::Parser;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Parser, Debug)]
pub struct ContributorsArgs {
    /// GitHub repository (owner/repo)
    #[arg(long, default_value = "librefang/librefang")]
    pub repo: String,

    /// Output path for contributors SVG
    #[arg(long, default_value = "web/public/assets/contributors.svg")]
    pub contributors_output: String,

    /// Output path for star history SVG
    #[arg(long, default_value = "web/public/assets/star-history.svg")]
    pub star_history_output: String,

    /// Only generate contributors SVG
    #[arg(long)]
    pub contributors_only: bool,

    /// Only generate star history SVG
    #[arg(long)]
    pub star_history_only: bool,

    /// Max number of contributors to display
    #[arg(long, default_value = "100")]
    pub max_contributors: usize,
}

fn gh_api(endpoint: &str) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let output = Command::new("gh")
        .args(["api", endpoint, "--paginate"])
        .output()?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh api failed: {}", err).into());
    }
    // gh --paginate may return multiple JSON arrays; merge them
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut all_items: Vec<serde_json::Value> = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let val: serde_json::Value = serde_json::from_str(trimmed)?;
        if let serde_json::Value::Array(arr) = val {
            all_items.extend(arr);
        } else {
            all_items.push(val);
        }
    }
    Ok(serde_json::Value::Array(all_items))
}

fn download_avatar_as_data_uri(avatar_url: &str) -> Option<String> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let url = format!("{}&s=96", avatar_url);
    let output = Command::new("curl")
        .args(["-fsSL", "--max-time", "10", &url])
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }
    let b64 = STANDARD.encode(&output.stdout);
    let mime = if output.stdout.starts_with(b"\x89PNG") {
        "image/png"
    } else if output.stdout.starts_with(b"\xff\xd8") {
        "image/jpeg"
    } else if output.stdout.starts_with(b"GIF") {
        "image/gif"
    } else {
        "image/png"
    };
    Some(format!("data:{};base64,{}", mime, b64))
}

fn generate_contributors_svg(
    repo: &str,
    output: &str,
    max_count: usize,
    root: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Fetching contributors for {}...", repo);
    let data = gh_api(&format!("repos/{}/contributors?per_page=100", repo))?;
    let contributors: Vec<&serde_json::Value> = data
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|c| c["type"].as_str() == Some("User"))
                .take(max_count)
                .collect()
        })
        .unwrap_or_default();

    println!("  Found {} contributors", contributors.len());

    let columns = 12usize;
    let cell = 54usize;
    let avatar_r = 24usize;
    let pad_x = 3usize;
    let pad_y = 3usize;
    let count = contributors.len();
    let cols = count.min(columns);
    let rows = if cols > 0 { count.div_ceil(cols) } else { 1 };
    let width = pad_x * 2 + cols * cell;
    let height = pad_y * 2 + rows * cell;

    let mut svg = String::new();
    svg.push_str(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{}" height="{}" viewBox="0 0 {} {}">"#,
        width, height, width, height
    ));
    svg.push_str("\n  <defs>\n");
    for i in 0..count {
        svg.push_str(&format!(
            "    <clipPath id=\"clip-{}\"><circle cx=\"0\" cy=\"0\" r=\"{}\"/></clipPath>\n",
            i, avatar_r
        ));
    }
    svg.push_str("  </defs>\n");

    for (i, c) in contributors.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        let cx = pad_x + col * cell + cell / 2;
        let cy = pad_y + row * cell + cell / 2;
        let login = c["login"].as_str().unwrap_or("?");
        let html_url = c["html_url"].as_str().unwrap_or("#");
        let avatar_url = c["avatar_url"].as_str().unwrap_or("");

        // Download avatar and embed as base64 data URI
        // (GitHub's CSP blocks external image loads inside SVGs)
        let avatar_src = match download_avatar_as_data_uri(avatar_url) {
            Some(data_uri) => data_uri,
            None => {
                eprintln!("  Warning: failed to download avatar for {}", login);
                format!("{}&s=96", avatar_url)
            }
        };

        svg.push_str(&format!(
            "  <a xlink:href=\"{}\" target=\"_blank\">\n",
            html_url
        ));
        svg.push_str(&format!("    <g transform=\"translate({},{})\">\n", cx, cy));
        svg.push_str(&format!(
            "      <image x=\"-{}\" y=\"-{}\" width=\"{}\" height=\"{}\" clip-path=\"url(#clip-{})\" xlink:href=\"{}\"/>\n",
            avatar_r, avatar_r, avatar_r * 2, avatar_r * 2, i, avatar_src
        ));
        svg.push_str(&format!(
            "      <circle cx=\"0\" cy=\"0\" r=\"{}\" fill=\"none\" stroke=\"#e1e4e8\" stroke-width=\"1\"/>\n",
            avatar_r
        ));
        svg.push_str(&format!("      <title>{}</title>\n", login));
        svg.push_str("    </g>\n");
        svg.push_str("  </a>\n");
    }

    svg.push_str("</svg>");

    let out_path = root.join(output);
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&out_path, &svg)?;
    println!("  Wrote {}", out_path.display());
    Ok(())
}

fn generate_star_history_svg(
    repo: &str,
    output: &str,
    root: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Fetching stargazers for {}...", repo);

    // Use gh api with star+json accept header for timestamps
    let out = Command::new("gh")
        .args([
            "api",
            &format!("repos/{}/stargazers?per_page=100", repo),
            "--paginate",
            "-H",
            "Accept: application/vnd.github.star+json",
        ])
        .output()?;

    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("gh api failed: {}", err).into());
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut stars: Vec<String> = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(arr) = val.as_array() {
                for item in arr {
                    if let Some(date) = item["starred_at"].as_str() {
                        // Extract just the date part (YYYY-MM-DD)
                        if date.len() >= 10 {
                            stars.push(date[..10].to_string());
                        }
                    }
                }
            }
        }
    }

    stars.sort();
    println!("  Found {} stars", stars.len());

    if stars.is_empty() {
        println!("  No stars found, skipping SVG generation");
        return Ok(());
    }

    // Build cumulative series by day
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut series: Vec<(String, usize)> = Vec::new();
    let mut running = 0usize;

    // Group by day
    let mut i = 0;
    while i < stars.len() {
        let day = &stars[i];
        let mut count = 0;
        while i < stars.len() && &stars[i] == day {
            count += 1;
            i += 1;
        }
        running += count;
        series.push((day.clone(), running));
    }

    // Ensure today is included
    if series.last().map(|s| s.0.as_str()) != Some(today.as_str()) {
        series.push((today.clone(), running));
    }

    let max_stars = series.last().map(|s| s.1).unwrap_or(0);
    let total_points = series.len();

    // SVG dimensions
    let width = 800.0f64;
    let height = 320.0f64;
    let left = 84.0;
    let right = 28.0;
    let top = 104.0;
    let bottom = 46.0;
    let chart_w = width - left - right;
    let chart_h = height - top - bottom;

    let x_for = |idx: usize| -> f64 {
        if total_points <= 1 {
            left + chart_w / 2.0
        } else {
            left + (idx as f64 / (total_points - 1) as f64) * chart_w
        }
    };
    let y_for = |stars: usize| -> f64 {
        if max_stars == 0 {
            top + chart_h
        } else {
            top + chart_h - (stars as f64 / max_stars as f64) * chart_h
        }
    };

    let points: String = series
        .iter()
        .enumerate()
        .map(|(i, (_, s))| format!("{:.2},{:.2}", x_for(i), y_for(*s)))
        .collect::<Vec<_>>()
        .join(" ");

    let area_path: String = {
        let mut d = format!("M {:.2},{:.2}", left, top + chart_h);
        for (i, (_, s)) in series.iter().enumerate() {
            d.push_str(&format!(" L {:.2},{:.2}", x_for(i), y_for(*s)));
        }
        d.push_str(&format!(" L {:.2},{:.2} Z", width - right, top + chart_h));
        d
    };

    let first_date = &series[0].0;
    let last_date = &series[series.len() - 1].0;
    let now_str = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC").to_string();

    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" role="img">
  <style>
    .bg {{ fill: #0f172a; }}
    .panel {{ fill: #111827; }}
    .grid {{ stroke: #243244; stroke-width: 1; }}
    .axis {{ stroke: #475569; stroke-width: 1.25; }}
    .line {{ fill: none; stroke: #22c55e; stroke-width: 3; stroke-linecap: round; stroke-linejoin: round; }}
    .area {{ fill: url(#areaGradient); }}
    .title {{ font: 700 22px ui-sans-serif, system-ui, sans-serif; fill: #f8fafc; }}
    .subtitle {{ font: 400 12px ui-sans-serif, system-ui, sans-serif; fill: #94a3b8; }}
    .axis-label {{ font: 400 11px ui-sans-serif, system-ui, sans-serif; fill: #94a3b8; }}
    .value {{ font: 700 28px ui-sans-serif, system-ui, sans-serif; fill: #f8fafc; }}
  </style>
  <defs>
    <linearGradient id="areaGradient" x1="0" x2="0" y1="0" y2="1">
      <stop offset="0%" stop-color="#22c55e" stop-opacity="0.35" />
      <stop offset="100%" stop-color="#22c55e" stop-opacity="0.02" />
    </linearGradient>
  </defs>
  <rect width="{width}" height="{height}" rx="18" class="bg" />
  <rect x="12" y="12" width="{w2}" height="{h2}" rx="14" class="panel" />
  <text x="30" y="52" class="title">Star History</text>
  <text x="30" y="74" class="subtitle">{repo}</text>
  <text x="{x_right}" y="52" class="value" text-anchor="end">{max_stars}</text>
  <text x="{x_right}" y="74" class="subtitle" text-anchor="end">Updated {now_str}</text>
  <line x1="{left}" y1="{y_bottom:.2}" x2="{x_end:.2}" y2="{y_bottom:.2}" class="axis" />
  <line x1="{left}" y1="{top:.2}" x2="{left}" y2="{y_bottom:.2}" class="axis" />
  <path d="{area_path}" class="area" />
  <polyline points="{points}" class="line" />
  <text x="{left}" y="{label_y}" class="axis-label" text-anchor="start">{first_date}</text>
  <text x="{x_end:.2}" y="{label_y}" class="axis-label" text-anchor="end">{last_date}</text>
  <text x="{y_label_x}" y="{y_0:.2}" class="axis-label" text-anchor="end">{max_stars}</text>
  <text x="{y_label_x}" y="{y_zero:.2}" class="axis-label" text-anchor="end">0</text>
</svg>"##,
        width = width,
        height = height,
        w2 = width - 24.0,
        h2 = height - 24.0,
        x_right = width - 30.0,
        x_end = width - right,
        y_bottom = top + chart_h,
        label_y = height - 14.0,
        y_label_x = left - 10.0,
        y_0 = y_for(max_stars) + 4.0,
        y_zero = y_for(0) + 4.0,
    );

    let out_path = root.join(output);
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&out_path, &svg)?;
    println!("  Wrote {}", out_path.display());
    Ok(())
}

pub fn run(args: ContributorsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    let gen_all = !args.contributors_only && !args.star_history_only;

    if gen_all || args.contributors_only {
        generate_contributors_svg(
            &args.repo,
            &args.contributors_output,
            args.max_contributors,
            &root,
        )?;
    }

    if gen_all || args.star_history_only {
        generate_star_history_svg(&args.repo, &args.star_history_output, &root)?;
    }

    println!("\nDone.");
    Ok(())
}
