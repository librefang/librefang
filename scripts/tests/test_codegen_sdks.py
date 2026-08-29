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
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "codegen-sdks.py"

def load_codegen_module():
    """Load the generator only when the standalone smoke test runs."""
    spec = importlib.util.spec_from_file_location("codegen_sdks", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load generator module from {SCRIPT}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def assert_in(needle, haystack, label):
    if needle not in haystack:
        raise AssertionError(f"FAIL [{label}]: substring not found:\n  {needle!r}")


def assert_not_in(needle, haystack, label):
    if needle in haystack:
        raise AssertionError(f"FAIL [{label}]: forbidden substring present:\n  {needle!r}")


def assert_matches(pattern, haystack, label):
    if re.search(pattern, haystack) is None:
        raise AssertionError(f"FAIL [{label}]: pattern not found:\n  {pattern!r}")


def expect(condition, label):
    if not condition:
        raise AssertionError(f"FAIL [{label}]")


def main():
    mod = load_codegen_module()
    tag_ops = mod.load_ops()

    tool_matches = [
        operation
        for operations in tag_ops.values()
        for operation in operations
        if operation["op_id"] == "invoke_tool"
    ]
    expect(len(tool_matches) == 1, f"expected one invoke_tool operation, got {len(tool_matches)}")
    tools = tool_matches[0]
    expect("agent_id" in tools["query_params"], f"expected agent_id query param, got {tools['query_params']}")
    expect(tools["has_body"], "invoke_tool should have body")

    agents_list = next((o for o in tag_ops.get("agents", []) if o["op_id"] == "list_agents"), None)
    expect(agents_list is not None, "list_agents missing from agents operations")
    expect(
        set(agents_list["query_params"]) == {"q", "status", "limit", "offset", "sort", "order"},
        f"unexpected list_agents query params: {agents_list['query_params']}",
    )

    stream_op = next((o for o in tag_ops.get("agents", []) if o["op_id"] == "send_message_stream"), None)
    expect(bool(stream_op and stream_op["is_stream"]), "send_message_stream not detected as stream")

    py = mod.gen_python(tag_ops)
    js = mod.gen_js(tag_ops)
    go = mod.gen_go(tag_ops)
    rs = mod.gen_rust(tag_ops)

    # invoke_tool signatures across SDKs
    assert_matches(r"def\s+invoke_tool\(\s*self,\s*name:\s*str,\s*agent_id:", py, "python-invoke_tool-sig")
    assert_matches(r"async\s+invokeTool\(\s*name,\s*data,\s*query\s*\)", js, "js-invoke_tool-sig")
    assert_matches(r"InvokeTool\(\s*name\s+string,\s*data\s+map\[string\]interface\{\},\s*query\s+map\[string\]string\s*\)", go, "go-invoke_tool-sig")
    assert_matches(r"pub\s+async\s+fn\s+invoke_tool\(\s*&self,\s*name:\s*&str,\s*data:\s*Value,\s*agent_id:\s*Option<&str>\s*\)", rs, "rust-invoke_tool-sig")
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
    expect(rs.count("_ = tx.closed() => return") == 3, "all stream network waits must cancel on receiver drop")
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
    expect(py.count("urlopen(req, timeout=self.timeout)") == 2, "both Python request paths must use configured timeout")
    # A stalled body read (timeout mid-stream, after urlopen() already succeeded) must be wrapped the same as a timeout during connection setup — not just the initial urlopen() call inside _stream.
    expect(py.count('raise LibreFangError(f"Request timed out after {self.timeout}s") from e') == 3, "all Python timeout paths must wrap consistently")
    assert_in('"error": fmt.Sprintf("new request: %v", err)', go, "go-stream-request-error")
    assert_not_in("req, _ := http.NewRequest", go, "go-no-discarded-stream-request-error")
    assert_in('buffer = b""', py, "python-byte-buffer")
    assert_in('lines = buffer.split(b"\\n")', py, "python-byte-line-split")
    # `.removesuffix("\r")`, not `.strip()`: an SSE `data:` value carries significant leading and
    # trailing whitespace, and #7203 changed the generator to trim only the CR of a CRLF line
    # ending. A `.strip()` here would silently corrupt multiline event payloads.
    assert_in('line = raw_line.decode().removesuffix("\\r")', py, "python-decode-complete-line")
    assert_not_in("line = line.decode().strip()", py, "python-no-whitespace-eating-decode")
    assert_not_in("buffer += chunk.decode()", py, "python-no-per-chunk-decode")
    assert_in('"error": fmt.Sprintf("marshal: %v", err)', go, "go-stream-marshal-error")
    assert_not_in("b, _ := json.Marshal(body)", go, "go-no-discarded-stream-marshal-error")
    assert_in("from urllib.error import HTTPError, URLError", py, "python-urlerror-import")
    expect(py.count("except URLError as e:") == 2, "both Python request paths must wrap connection failures")
    assert_in("active_error = sys.exc_info()[0] is not None", py, "python-stream-close-finally")
    assert_in("if buffer:", py, "python-flush-trailing-sse-line")
    assert_in('line = buffer.decode().removesuffix("\\r")', py, "python-parse-trailing-sse-line")
    expect(
        py.count('line = raw_line.decode().removesuffix("\\r")')
        + py.count('line = buffer.decode().removesuffix("\\r")')
        == 2,
        "trailing SSE flush must decode strictly, matching per-line decode",
    )
    assert_in("const trailing = buffer.trim();", js, "js-flush-trailing-sse-line")
    assert_in('if (trailing.startsWith("data: ")) {', js, "js-parse-trailing-sse-line")

    # SSE line-size cap
    assert_in("MAX_SSE_LINE", rs, "rust-max-sse")
    assert_in("maxSSELine", go, "go-max-sse")

    # Reserved-word escape works
    expect(mod._py_safe("class") == "class_", "Python reserved-word escape")
    expect(mod._rust_safe("type") == "type_", "Rust reserved-word escape")
    expect(
        mod._rust_path_segments("/api/agents/{id}", owned=False)
        == '&["api", "agents", id]',
        "borrowed Rust path segments",
    )
    expect(
        mod._rust_path_segments("/api/agents/{id}", owned=True)
        == 'vec!["api".to_string(), "agents".to_string(), id.to_string()]',
        "owned Rust path segments",
    )

    print(f"OK — {sum(len(v) for v in tag_ops.values())} ops across {len(tag_ops)} tags")


if __name__ == "__main__":
    main()
