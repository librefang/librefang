use clap::Parser;
use std::fs;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Parser, Debug)]
pub struct IntegrationTestArgs {
    /// GROQ_API_KEY for LLM tests
    #[arg(long)]
    pub api_key: Option<String>,

    /// Port for the daemon (default: 4545)
    #[arg(long, default_value = "4545")]
    pub port: u16,

    /// Skip LLM integration test
    #[arg(long)]
    pub skip_llm: bool,

    /// Path to the librefang binary
    #[arg(long)]
    pub binary: Option<String>,
}

fn repo_root() -> PathBuf {
    let mut dir = std::env::current_dir().expect("cannot get cwd");
    loop {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.exists() {
            let content = fs::read_to_string(&cargo_toml).unwrap_or_default();
            if content.contains("[workspace]") {
                return dir;
            }
        }
        if !dir.pop() {
            panic!("could not find workspace root (no Cargo.toml with [workspace])");
        }
    }
}

fn default_binary(root: &Path) -> PathBuf {
    if cfg!(target_os = "windows") {
        root.join("target/release/librefang.exe")
    } else {
        root.join("target/release/librefang")
    }
}

fn kill_process_on_port(port: u16) {
    if cfg!(target_os = "windows") {
        let output = Command::new("cmd")
            .args(["/C", &format!("netstat -ano | findstr :{}", port)])
            .output();
        if let Ok(output) = output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let Some(pid) = parts.last() {
                    let _ = Command::new("taskkill").args(["/PID", pid, "/F"]).output();
                }
            }
        }
    } else {
        let output = Command::new("lsof")
            .args(["-ti", &format!(":{}", port)])
            .output();
        if let Ok(output) = output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for pid in stdout.lines() {
                let pid = pid.trim();
                if !pid.is_empty() {
                    let _ = Command::new("kill").args(["-9", pid]).output();
                }
            }
        }
    }
}

fn cleanup_daemon(daemon: &mut Child, port: u16) {
    println!("Cleaning up: killing daemon (PID: {})...", daemon.id());
    let _ = daemon.kill();
    let _ = daemon.wait();
    kill_process_on_port(port);
}

fn wait_for_health(port: u16, timeout: Duration) -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();
    let url = format!("http://127.0.0.1:{}/api/health", port);

    while start.elapsed() < timeout {
        if TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok() {
            let output = Command::new("curl")
                .args(["-s", "-o", "/dev/null", "-w", "%{http_code}", &url])
                .output();
            if let Ok(output) = output {
                let code = String::from_utf8_lossy(&output.stdout);
                if code.trim() == "200" {
                    return Ok(());
                }
            }
        }
        thread::sleep(Duration::from_millis(500));
    }
    Err(format!("Health check timed out after {}s", timeout.as_secs()).into())
}

fn http_get(port: u16, path: &str) -> Result<(u16, String), Box<dyn std::error::Error>> {
    let url = format!("http://127.0.0.1:{}{}", port, path);
    let output = Command::new("curl")
        .args(["-s", "-w", "\n%{http_code}", &url])
        .output()?;
    let full = String::from_utf8_lossy(&output.stdout).to_string();
    let lines: Vec<&str> = full.trim_end().rsplitn(2, '\n').collect();
    if lines.len() >= 2 {
        let code: u16 = lines[0].trim().parse().unwrap_or(0);
        let body = lines[1].to_string();
        Ok((code, body))
    } else {
        Err("unexpected curl output".into())
    }
}

fn http_post(
    port: u16,
    path: &str,
    body: &str,
) -> Result<(u16, String), Box<dyn std::error::Error>> {
    let url = format!("http://127.0.0.1:{}{}", port, path);
    let output = Command::new("curl")
        .args([
            "-s",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "-d",
            body,
            "-w",
            "\n%{http_code}",
            &url,
        ])
        .output()?;
    let full = String::from_utf8_lossy(&output.stdout).to_string();
    let lines: Vec<&str> = full.trim_end().rsplitn(2, '\n').collect();
    if lines.len() >= 2 {
        let code: u16 = lines[0].trim().parse().unwrap_or(0);
        let resp_body = lines[1].to_string();
        Ok((code, resp_body))
    } else {
        Err("unexpected curl output".into())
    }
}

struct TestResults {
    passed: usize,
    failed: usize,
    errors: Vec<String>,
}

impl TestResults {
    fn new() -> Self {
        Self {
            passed: 0,
            failed: 0,
            errors: Vec::new(),
        }
    }

    fn pass(&mut self, name: &str) {
        println!("  PASS: {}", name);
        self.passed += 1;
    }

    fn fail(&mut self, name: &str, reason: &str) {
        println!("  FAIL: {} -- {}", name, reason);
        self.failed += 1;
        self.errors.push(format!("{}: {}", name, reason));
    }
}

pub fn run(args: IntegrationTestArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    let binary = match &args.binary {
        Some(b) => PathBuf::from(b),
        None => default_binary(&root),
    };

    if !binary.exists() {
        return Err(format!(
            "Binary not found: {} -- build with `cargo build --release -p librefang-cli` first",
            binary.display()
        )
        .into());
    }

    let port = args.port;
    let mut results = TestResults::new();

    // Step 1: Kill any existing process on the port
    println!("Killing any existing process on port {}...", port);
    kill_process_on_port(port);
    thread::sleep(Duration::from_secs(2));

    // Step 2: Start daemon
    println!("Starting daemon: {} start", binary.display());
    let mut daemon = {
        let mut cmd = Command::new(&binary);
        cmd.arg("start")
            .current_dir(&root)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(ref key) = args.api_key {
            cmd.env("GROQ_API_KEY", key);
        }
        cmd.spawn()?
    };

    println!("  Daemon started (PID: {})", daemon.id());

    // Wait for health
    println!("Waiting for health endpoint (up to 10s)...");
    if let Err(e) = wait_for_health(port, Duration::from_secs(10)) {
        cleanup_daemon(&mut daemon, port);
        return Err(format!("Daemon failed to start: {}", e).into());
    }
    println!("  Daemon is healthy");

    // Step 3: Test basic endpoints
    println!();
    println!("Running endpoint tests...");

    // GET /api/health
    match http_get(port, "/api/health") {
        Ok((code, _)) if code == 200 => results.pass("GET /api/health"),
        Ok((code, body)) => results.fail(
            "GET /api/health",
            &format!("status={}, body={}", code, body),
        ),
        Err(e) => results.fail("GET /api/health", &e.to_string()),
    }

    // GET /api/agents
    let mut first_agent_id: Option<String> = None;
    match http_get(port, "/api/agents") {
        Ok((code, body)) if code == 200 => {
            results.pass("GET /api/agents");
            if let Ok(agents) = serde_json::from_str::<serde_json::Value>(&body) {
                if let Some(arr) = agents.as_array() {
                    if let Some(first) = arr.first() {
                        if let Some(id) = first["id"].as_str() {
                            first_agent_id = Some(id.to_string());
                        }
                    }
                }
            }
        }
        Ok((code, body)) => results.fail(
            "GET /api/agents",
            &format!("status={}, body={}", code, body),
        ),
        Err(e) => results.fail("GET /api/agents", &e.to_string()),
    }

    // GET /api/budget
    match http_get(port, "/api/budget") {
        Ok((code, _)) if code == 200 => results.pass("GET /api/budget"),
        Ok((code, body)) => results.fail(
            "GET /api/budget",
            &format!("status={}, body={}", code, body),
        ),
        Err(e) => results.fail("GET /api/budget", &e.to_string()),
    }

    // GET /api/network/status
    match http_get(port, "/api/network/status") {
        Ok((code, _)) if code == 200 => results.pass("GET /api/network/status"),
        Ok((code, body)) => results.fail(
            "GET /api/network/status",
            &format!("status={}, body={}", code, body),
        ),
        Err(e) => results.fail("GET /api/network/status", &e.to_string()),
    }

    // Step 4: LLM test (unless --skip-llm)
    if !args.skip_llm {
        println!();
        println!("Running LLM integration test...");

        if args.api_key.is_none() {
            results.fail("LLM test", "no --api-key provided");
        } else if let Some(ref agent_id) = first_agent_id {
            let payload = r#"{"message": "Say hello in 5 words."}"#;
            match http_post(port, &format!("/api/agents/{}/message", agent_id), payload) {
                Ok((code, _)) if code == 200 => {
                    results.pass("POST /api/agents/{id}/message");

                    // Verify budget updated
                    match http_get(port, "/api/budget") {
                        Ok((200, body)) => {
                            if let Ok(budget) = serde_json::from_str::<serde_json::Value>(&body) {
                                let cost = budget["total_cost"]
                                    .as_f64()
                                    .or_else(|| budget["spent"].as_f64())
                                    .unwrap_or(0.0);
                                if cost > 0.0 {
                                    results.pass("Budget updated after LLM call");
                                } else {
                                    results
                                        .fail("Budget updated after LLM call", "cost is still 0");
                                }
                            } else {
                                results.fail(
                                    "Budget updated after LLM call",
                                    "could not parse budget response",
                                );
                            }
                        }
                        Ok((code, body)) => results.fail(
                            "Budget updated after LLM call",
                            &format!("budget GET status={}, body={}", code, body),
                        ),
                        Err(e) => results.fail("Budget updated after LLM call", &e.to_string()),
                    }
                }
                Ok((code, body)) => results.fail(
                    "POST /api/agents/{id}/message",
                    &format!("status={}, body={}", code, body),
                ),
                Err(e) => results.fail("POST /api/agents/{id}/message", &e.to_string()),
            }
        } else {
            results.fail("LLM test", "no agents available to test with");
        }
    }

    // Step 5: Cleanup
    println!();
    cleanup_daemon(&mut daemon, port);

    // Summary
    println!();
    println!(
        "Results: {} passed, {} failed",
        results.passed, results.failed
    );

    if results.failed > 0 {
        println!();
        println!("Failures:");
        for err in &results.errors {
            println!("  - {}", err);
        }
        Err(format!("{} test(s) failed", results.failed).into())
    } else {
        println!("All integration tests passed!");
        Ok(())
    }
}
