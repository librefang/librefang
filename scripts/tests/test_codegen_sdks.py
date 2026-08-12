#!/usr/bin/env python3
"""Smoke test for scripts/codegen-sdks.py.

Runs against the real openapi.json and asserts a few invariants that have
historically regressed:
- SSE detection handles both content-type and operationId suffix.
- Query params on invoke_tool / list_agents flow into every SDK surface.
- Stream code uses buffered parsing (no bare chunk-split) and surfaces errors.

Run: python3 scripts/tests/test_codegen_sdks.py
"""
import importlib.util
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "codegen-sdks.py"

spec = importlib.util.spec_from_file_location("codegen_sdks", SCRIPT)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)


def assert_in(needle, haystack, label):
    if needle not in haystack:
        print(f"FAIL [{label}]: substring not found:\n  {needle!r}", file=sys.stderr)
        sys.exit(1)


def assert_not_in(needle, haystack, label):
    if needle in haystack:
        print(f"FAIL [{label}]: forbidden substring present:\n  {needle!r}", file=sys.stderr)
        sys.exit(1)


def main():
    tag_ops = mod.load_ops()

    tools = None
    for ops in tag_ops.values():
        for o in ops:
            if o["op_id"] == "invoke_tool":
                tools = o
                break
    assert tools is not None, "invoke_tool missing from loaded ops"
    assert "agent_id" in tools["query_params"], f"expected agent_id query param, got {tools['query_params']}"
    assert tools["has_body"], "invoke_tool should have body"

    agents_list = next((o for o in tag_ops.get("agents", []) if o["op_id"] == "list_agents"), None)
    assert agents_list is not None
    assert set(agents_list["query_params"]) == {"q", "status", "limit", "offset", "sort", "order"}

    stream_op = next((o for o in tag_ops.get("agents", []) if o["op_id"] == "send_message_stream"), None)
    assert stream_op and stream_op["is_stream"], "send_message_stream not detected as stream"

    py = mod.gen_python(tag_ops)
    js = mod.gen_js(tag_ops)
    go = mod.gen_go(tag_ops)
    rs = mod.gen_rust(tag_ops)

    # invoke_tool signatures across SDKs
    assert_in("def invoke_tool(self, name: str, agent_id:", py, "python-invoke_tool-sig")
    assert_in("async invokeTool(name, data, query)", js, "js-invoke_tool-sig")
    assert_in("InvokeTool(name string, data map[string]interface{}, query map[string]string)", go, "go-invoke_tool-sig")
    assert_in("pub async fn invoke_tool(&self, name: &str, data: Value, agent_id: Option<&str>)", rs, "rust-invoke_tool-sig")
    assert_in('#[tokio::main(flavor = "current_thread")]', rs, "rust-doc-current-thread-runtime")
    assert_in("Self::with_client(base_url, client)", rs, "rust-default-client-delegation")
    assert_in(".connect_timeout(DEFAULT_CONNECT_TIMEOUT)", rs, "rust-default-client-connect-timeout")
    assert_in("pub fn with_client(base_url: impl Into<String>, client: Client) -> Self", rs, "rust-custom-client-constructor")

    # Stream correctness
    assert_in("bufio.NewReaderSize", go, "go-bufio-reader")
    assert_not_in('strings.Split(string(buf[:n])', go, "go-no-bare-split")
    assert_in("Vec<u8>", rs, "rust-byte-buffer")
    assert_not_in("from_utf8_lossy(&chunk)", rs, "rust-no-lossy-chunk")
    assert_in('"status": status', rs, "rust-error-event-status")
    assert_in(".timeout(DEFAULT_REQUEST_TIMEOUT)", rs, "rust-request-timeout")
    assert_in("mpsc::channel(STREAM_CHANNEL_CAPACITY)", rs, "rust-bounded-stream-channel")
    assert_not_in("mpsc::unbounded_channel()", rs, "rust-no-unbounded-stream-channel")
    assert rs.count("_ = tx.closed() => return") == 3, "all stream network waits must cancel on receiver drop"
    assert_in("Some(Err(e)) => {", rs, "rust-stream-result-loop")
    assert_in(".path_segments_mut()", rs, "rust-url-segment-builder")
    assert_in('&["api", "agents", id]', rs, "rust-borrowed-path-segments")
    assert_in('id.to_string(),', rs, "rust-owned-stream-path-segment")
    assert_not_in('format!("/api/', rs, "rust-no-raw-path-formatting")
    assert_in('"error": format!("stream error: {}", e)', rs, "rust-stream-transport-error")
    assert_not_in("while let Some(Ok(chunk))", rs, "rust-no-silent-stream-error")
    assert_in('"status": resp.StatusCode', go, "go-error-event-status")
    assert_in("if !buffer.is_empty()", rs, "rust-flush-trailing-sse-line")
    assert_in('if let Some(data) = line.trim().strip_prefix("data: ")', rs, "rust-parse-trailing-sse-line")
    assert_in(
        'invalid utf-8 in SSE line at byte {}", e.valid_up_to())',
        rs,
        "rust-trailing-sse-flush-reports-invalid-utf8",
    )
    assert_in("DEFAULT_TIMEOUT = 30.0", py, "python-default-timeout")
    assert py.count("urlopen(req, timeout=self.timeout)") == 2
    # A stalled body read (timeout mid-stream, after urlopen() already succeeded) must be wrapped the same as a timeout during connection setup — not just the initial urlopen() call inside _stream.
    assert py.count('raise LibreFangError(f"Request timed out after {self.timeout}s") from e') == 3
    assert_in('"error": fmt.Sprintf("new request: %v", err)', go, "go-stream-request-error")
    assert_not_in("req, _ := http.NewRequest", go, "go-no-discarded-stream-request-error")
    assert_in('buffer = b""', py, "python-byte-buffer")
    assert_in('lines = buffer.split(b"\\n")', py, "python-byte-line-split")
    assert_in("line = line.decode().strip()", py, "python-decode-complete-line")
    assert_not_in("buffer += chunk.decode()", py, "python-no-per-chunk-decode")
    assert_in('"error": fmt.Sprintf("marshal: %v", err)', go, "go-stream-marshal-error")
    assert_not_in("b, _ := json.Marshal(body)", go, "go-no-discarded-stream-marshal-error")
    assert_in("from urllib.error import HTTPError, URLError", py, "python-urlerror-import")
    assert py.count("except URLError as e:") == 2, "both Python request paths must wrap connection failures"
    assert_in("active_error = sys.exc_info()[0] is not None", py, "python-stream-close-finally")
    assert_in("if buffer:", py, "python-flush-trailing-sse-line")
    assert_in("line = buffer.decode().strip()", py, "python-parse-trailing-sse-line")
    assert py.count("line = line.decode().strip()") + py.count(
        "line = buffer.decode().strip()"
    ) == 2, "trailing SSE flush must decode strictly, matching the per-line decode above it"
    assert_in("const trailing = buffer.trim();", js, "js-flush-trailing-sse-line")
    assert_in('if (trailing.startsWith("data: ")) {', js, "js-parse-trailing-sse-line")

    # SSE line-size cap
    assert_in("MAX_SSE_LINE", rs, "rust-max-sse")
    assert_in("maxSSELine", go, "go-max-sse")

    # Reserved-word escape works
    assert mod._py_safe("class") == "class_"
    assert mod._rust_safe("type") == "type_"
    assert mod._rust_path_segments("/api/agents/{id}", owned=False) == '&["api", "agents", id]'
    assert mod._rust_path_segments("/api/agents/{id}", owned=True) == (
        'vec!["api".to_string(), "agents".to_string(), id.to_string()]'
    )

    print(f"OK — {sum(len(v) for v in tag_ops.values())} ops across {len(tag_ops)} tags")


if __name__ == "__main__":
    main()
