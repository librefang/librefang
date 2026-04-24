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

    # Stream correctness
    assert_in("bufio.NewReaderSize", go, "go-bufio-reader")
    assert_not_in('strings.Split(string(buf[:n])', go, "go-no-bare-split")
    assert_in("Vec<u8>", rs, "rust-byte-buffer")
    assert_not_in("from_utf8_lossy(&chunk)", rs, "rust-no-lossy-chunk")
    assert_in('"status": status', rs, "rust-error-event-status")
    assert_in('"status": resp.StatusCode', go, "go-error-event-status")

    # SSE line-size cap
    assert_in("MAX_SSE_LINE", rs, "rust-max-sse")
    assert_in("maxSSELine", go, "go-max-sse")

    # Reserved-word escape works
    assert mod._py_safe("class") == "class_"
    assert mod._rust_safe("type") == "type_"

    print(f"OK — {sum(len(v) for v in tag_ops.values())} ops across {len(tag_ops)} tags")


if __name__ == "__main__":
    main()
