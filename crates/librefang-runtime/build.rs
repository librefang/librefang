use std::path::PathBuf;
use std::time::Duration;

const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=LIBREFANG_SKIP_OPENROUTER_BUILD_SNAPSHOT");

    let output = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is set"))
        .join("openrouter-models.json");
    let skip = std::env::var_os("LIBREFANG_SKIP_OPENROUTER_BUILD_SNAPSHOT").is_some();

    let body = (!skip)
        .then(fetch_snapshot)
        .flatten()
        .unwrap_or_else(|| r#"{"data":[]}"#.to_string());

    std::fs::write(output, body).expect("write OpenRouter build snapshot");
}

fn fetch_snapshot() -> Option<String> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .user_agent("LibreFang build snapshot")
        .build();
    let agent: ureq::Agent = config.into();
    let mut response = agent.get(OPENROUTER_MODELS_URL).call().ok()?;
    let body = response.body_mut().read_to_string().ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&body).ok()?;
    let models = parsed.get("data")?.as_array()?;
    (!models.is_empty()).then_some(body)
}
