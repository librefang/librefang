#!/usr/bin/env python3
"""
codegen-sdks: auto-generates Python, JS, Go, and Rust SDKs from openapi.json.

Usage:
    python3 scripts/codegen-sdks.py           # regenerate all SDKs
    python3 scripts/codegen-sdks.py --dry-run # print diffs, don't write
"""
import json
import sys
import re
import shutil
import subprocess
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).parent.parent
OPENAPI = ROOT / "openapi.json"

# Tags to skip entirely (OpenAI compat endpoints — not part of our public SDK)
SKIP_TAGS = {"openai"}

# Paths that don't start with /api/ are skipped (well-known, a2a server-side, etc.)
API_PREFIX = "/api/"


# ── helpers ──────────────────────────────────────────────────────────────────

def _path_params(path: str) -> list:
    return re.findall(r"\{(\w+)\}", path)

def _tag_attr(tag: str) -> str:
    """'proactive-memory' → 'proactive_memory'"""
    return tag.replace("-", "_")

def _tag_pascal(tag: str) -> str:
    """'proactive-memory' or 'auto_dream' → 'ProactiveMemory' / 'AutoDream'"""
    return "".join(p.title() for p in re.split(r"[-_]", tag))

def _op_camel(op_id: str) -> str:
    """'list_agent_sessions' → 'listAgentSessions'"""
    parts = op_id.split("_")
    return parts[0] + "".join(p.title() for p in parts[1:])

def _op_pascal(op_id: str) -> str:
    """'list_agent_sessions' → 'ListAgentSessions'"""
    return "".join(p.title() for p in op_id.split("_"))

def _is_stream(op: dict) -> bool:
    for _, resp in op.get("responses", {}).items():
        for ct in resp.get("content", {}):
            if "event-stream" in ct:
                return True
    # Fallback: operationId or path ending in /stream
    op_id = op.get("operationId", "")
    return op_id.endswith("_stream") or op_id.endswith("stream")

def _has_body(op: dict, method: str) -> bool:
    return method in ("post", "put", "patch") and bool(op.get("requestBody"))

def _py_path(path: str) -> str:
    """'/api/agents/{id}' → f-string body '/api/agents/{id}'"""
    return path  # same syntax for Python f-strings

def _go_path(path: str) -> str:
    """'/api/agents/{id}/sessions/{session_id}' → '/api/agents/%s/sessions/%s'"""
    return re.sub(r"\{[^}]+\}", "%s", path)

def _js_path(path: str) -> str:
    """'/api/agents/{id}' → template literal body '/api/agents/${id}'"""
    return re.sub(r"\{(\w+)\}", r"${\1}", path)

# Reserved identifiers by language — append trailing underscore to avoid collisions.
_PY_RESERVED = {"class", "from", "import", "return", "lambda", "global", "None", "True", "False",
                "and", "or", "not", "if", "else", "for", "while", "pass", "yield", "def"}
_RUST_RESERVED = {"type", "match", "move", "mod", "ref", "trait", "impl", "use", "let",
                  "self", "Self", "super", "crate", "fn", "if", "else", "for", "while",
                  "loop", "return", "struct", "enum", "const", "static", "unsafe", "async",
                  "await", "dyn", "where", "true", "false", "as", "in", "box", "pub"}

def _py_safe(name: str) -> str:
    return name + "_" if name in _PY_RESERVED else name

def _rust_safe(name: str) -> str:
    return name + "_" if name in _RUST_RESERVED else name


# ── load operations ───────────────────────────────────────────────────────────

def _query_params(op: dict) -> list:
    """Extract ?name=... query parameter names from an operation."""
    return [p["name"] for p in op.get("parameters", []) if p.get("in") == "query"]


def load_ops() -> dict:
    data = json.loads(OPENAPI.read_text())
    tag_ops: dict = defaultdict(list)
    seen: set = set()

    for path, methods in sorted(data["paths"].items()):
        if not path.startswith(API_PREFIX):
            continue
        for method, op in methods.items():
            if method not in ("get", "post", "put", "patch", "delete"):
                continue
            op_id = op.get("operationId", "")
            if not op_id:
                continue
            if op_id in seen:
                print(f"warning: duplicate operationId '{op_id}' at {method.upper()} {path} — skipped", file=sys.stderr)
                continue
            seen.add(op_id)

            for tag in op.get("tags", ["system"]):
                if tag in SKIP_TAGS:
                    continue
                tag_ops[tag].append({
                    "http": method.upper(),
                    "path": path,
                    "op_id": op_id,
                    "params": _path_params(path),
                    "query_params": _query_params(op),
                    "has_body": _has_body(op, method),
                    "is_stream": _is_stream(op),
                })
    return dict(tag_ops)


# ── Python generator ──────────────────────────────────────────────────────────

_PY_STATIC = '''\
"""
LibreFang Python Client — AUTO-GENERATED from openapi.json.
Do not edit manually. Run: python3 scripts/codegen-sdks.py

Usage:
    from librefang_client import LibreFang

    client = LibreFang("http://localhost:4545")
    agents = client.agents.list_agents()

    for event in client.agents.send_message_stream(agent_id, message="Hello"):
        if event.get("type") == "text_delta":
            print(event["delta"], end="", flush=True)
"""

import json
import socket
import sys
from typing import Any, Dict, Generator, Optional
from urllib.request import urlopen, Request
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode

DEFAULT_TIMEOUT = 30.0


class LibreFangError(Exception):
    def __init__(self, message: str, status: int = 0, body: str = ""):
        super().__init__(message)
        self.status = status
        self.body = body


class _Resource:
    def __init__(self, client: "LibreFang"):
        self._c = client


class LibreFang:
    """LibreFang REST API client. Zero dependencies — uses only stdlib urllib."""

    def __init__(self, base_url: str, headers: Optional[Dict[str, str]] = None, timeout: float = DEFAULT_TIMEOUT):
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout
        self._headers = {"Content-Type": "application/json"}
        if headers:
            self._headers.update(headers)
{resource_init}
    def _request(self, method: str, path: str, body: Any = None, query: Optional[Dict[str, Any]] = None) -> Any:
        url = self.base_url + path
        if query:
            filtered = {k: v for k, v in query.items() if v is not None}
            if filtered:
                url += ("&" if "?" in url else "?") + urlencode(filtered, doseq=True)
        data = json.dumps(body).encode() if body is not None else None
        req = Request(url, data=data, headers=self._headers, method=method)
        try:
            with urlopen(req, timeout=self.timeout) as resp:
                ct = resp.headers.get("content-type", "")
                text = resp.read().decode()
                if "application/json" in ct:
                    return json.loads(text)
                return text
        except HTTPError as e:
            body_text = e.read().decode() if e.fp else ""
            raise LibreFangError(f"HTTP {e.code}: {body_text}", e.code, body_text) from e
        except socket.timeout as e:
            raise LibreFangError(f"Request timed out after {self.timeout}s") from e
        except URLError as e:
            raise LibreFangError(f"Connection error: {e.reason}") from e

    def _stream(self, method: str, path: str, body: Any = None, query: Optional[Dict[str, Any]] = None) -> Generator[Dict, None, None]:
        """SSE streaming — yields parsed JSON events."""
        url = self.base_url + path
        if query:
            filtered = {k: v for k, v in query.items() if v is not None}
            if filtered:
                url += ("&" if "?" in url else "?") + urlencode(filtered, doseq=True)
        data = json.dumps(body).encode() if body is not None else None
        headers = dict(self._headers)
        headers["Accept"] = "text/event-stream"
        req = Request(url, data=data, headers=headers, method=method)
        try:
            resp = urlopen(req, timeout=self.timeout)
        except HTTPError as e:
            body_text = e.read().decode() if e.fp else ""
            raise LibreFangError(f"HTTP {e.code}: {body_text}", e.code, body_text) from e
        except socket.timeout as e:
            raise LibreFangError(f"Request timed out after {self.timeout}s") from e
        except URLError as e:
            raise LibreFangError(f"Connection error: {e.reason}") from e

        try:
            buffer = b""
            data_lines = []
            while True:
                chunk = resp.read(4096)
                if not chunk:
                    break
                buffer += chunk
                lines = buffer.split(b"\\n")
                buffer = lines.pop()
                for raw_line in lines:
                    line = raw_line.decode().removesuffix("\\r")
                    if not line:
                        if not data_lines:
                            continue
                        data_str = "\\n".join(data_lines)
                        data_lines.clear()
                        if data_str == "[DONE]":
                            return
                        try:
                            yield json.loads(data_str)
                        except json.JSONDecodeError:
                            yield {"raw": data_str}
                    elif line.startswith("data:"):
                        value = line[5:]
                        if value.startswith(" "):
                            value = value[1:]
                        data_lines.append(value)
            # A clean EOF can arrive without a trailing newline, leaving the last event in the buffer.
            # Parse it here rather than dropping it; the loop above only fires on a newline.
            if buffer:
                line = buffer.decode().removesuffix("\\r")
                if line.startswith("data:"):
                    value = line[5:]
                    if value.startswith(" "):
                        value = value[1:]
                    data_lines.append(value)
            if data_lines:
                data_str = "\\n".join(data_lines)
                if data_str != "[DONE]":
                    try:
                        yield json.loads(data_str)
                    except json.JSONDecodeError:
                        yield {"raw": data_str}
        except socket.timeout as e:
            raise LibreFangError(f"Request timed out after {self.timeout}s") from e
        finally:
            active_error = sys.exc_info()[0] is not None
            try:
                resp.close()
            except Exception:
                if not active_error:
                    raise

'''


def gen_python(tag_ops: dict) -> str:
    tags = sorted(tag_ops)
    init_lines = []
    for tag in tags:
        attr = _tag_attr(tag)
        cls = f"_{_tag_pascal(tag)}Resource"
        init_lines.append(f"        self.{attr} = {cls}(self)")
    resource_init = "\n".join(init_lines) + "\n\n"

    out = _PY_STATIC.replace("{resource_init}", resource_init)

    for tag in tags:
        ops = tag_ops[tag]
        cls = f"_{_tag_pascal(tag)}Resource"
        dashes = "─" * max(1, 50 - len(tag))
        out += f"\n# ── {_tag_pascal(tag)} Resource {dashes}\n\n"
        out += f"class {cls}(_Resource):\n"

        for op in ops:
            op_id = op["op_id"]
            params = op["params"]
            query_params = op["query_params"]
            has_body = op["has_body"]
            is_stream = op["is_stream"]
            http = op["http"]
            path = op["path"]

            sig_parts = ["self"] + [f"{p}: str" for p in params]
            for qp in query_params:
                sig_parts.append(f"{_py_safe(qp)}: Any = None")
            if has_body:
                sig_parts.append("**data")

            sig = ", ".join(sig_parts)
            path_expr = f'f"{_py_path(path)}"' if params else f'"{path}"'

            ret_type = " -> Generator[Dict, None, None]" if is_stream else ""

            body_arg = "data" if has_body else "None"
            if query_params:
                q_items = ", ".join(f'"{qp}": {_py_safe(qp)}' for qp in query_params)
                query_arg = f", query={{{q_items}}}"
            else:
                query_arg = ""

            out += f"\n    def {op_id}({sig}){ret_type}:\n"
            call = "_stream" if is_stream else "_request"
            if has_body or query_params:
                out += f'        return self._c.{call}("{http}", {path_expr}, {body_arg}{query_arg})\n'
            else:
                out += f'        return self._c.{call}("{http}", {path_expr})\n'

        out += "\n"

    return out


# ── JavaScript generator ──────────────────────────────────────────────────────

_JS_STATIC = """\
/**
 * @librefang/sdk — AUTO-GENERATED from openapi.json.
 * Do not edit manually. Run: python3 scripts/codegen-sdks.py
 *
 * Usage:
 *   const { LibreFang } = require("@librefang/sdk");
 *   const client = new LibreFang("http://localhost:4545");
 *
 *   const agents = await client.agents.listAgents();
 *
 *   // Streaming:
 *   for await (const event of client.agents.sendMessageStream(agentId, { message: "Hello" })) {
 *     process.stdout.write(event.delta || "");
 *   }
 */

"use strict";

class LibreFangError extends Error {
  constructor(message, status, body) {
    super(message);
    this.name = "LibreFangError";
    this.status = status;
    this.body = body;
  }
}

class LibreFang {
  constructor(baseUrl, opts) {
    this.baseUrl = baseUrl.replace(/\\/+$/, "");
    this._headers = Object.assign({ "Content-Type": "application/json" }, (opts && opts.headers) || {});
{resource_init}
  }

  _withQuery(path, query) {
    if (!query) return path;
    const params = new URLSearchParams();
    for (const [k, v] of Object.entries(query)) {
      if (v === undefined || v === null) continue;
      params.append(k, String(v));
    }
    const q = params.toString();
    if (!q) return path;
    return path + (path.includes("?") ? "&" : "?") + q;
  }

  async _request(method, path, body, query) {
    const url = this.baseUrl + this._withQuery(path, query);
    const opts = { method, headers: this._headers };
    if (body !== undefined && body !== null) opts.body = JSON.stringify(body);
    const res = await fetch(url, opts);
    const text = await res.text();
    if (!res.ok) throw new LibreFangError(`HTTP ${res.status}: ${text}`, res.status, text);
    const ct = res.headers.get("content-type") || "";
    return ct.includes("application/json") ? JSON.parse(text) : text;
  }

  async *_stream(method, path, body, query) {
    const url = this.baseUrl + this._withQuery(path, query);
    const headers = Object.assign({}, this._headers, { Accept: "text/event-stream" });
    const opts = { method, headers };
    if (body !== undefined && body !== null) opts.body = JSON.stringify(body);
    const res = await fetch(url, opts);
    if (!res.ok) {
      const text = await res.text();
      throw new LibreFangError(`HTTP ${res.status}: ${text}`, res.status, text);
    }
    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split("\\n");
      buffer = lines.pop();
      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed.startsWith("data: ")) continue;
        const data = trimmed.slice(6);
        if (data === "[DONE]") return;
        try { yield JSON.parse(data); } catch { yield { raw: data }; }
      }
    }
    // A clean EOF can arrive without a trailing newline, leaving the last event in the buffer.
    // Parse it here rather than dropping it; the loop above only fires on a newline.
    const trailing = buffer.trim();
    if (trailing.startsWith("data: ")) {
      const data = trailing.slice(6);
      if (data !== "[DONE]") {
        try { yield JSON.parse(data); } catch { yield { raw: data }; }
      }
    }
  }
}

"""


def gen_js(tag_ops: dict) -> str:
    tags = sorted(tag_ops)
    init_lines = []
    for tag in tags:
        attr = _tag_attr(tag)
        cls = f"{_tag_pascal(tag)}Resource"
        init_lines.append(f"    this.{attr} = new {cls}(this);")
    resource_init = "\n".join(init_lines)

    out = _JS_STATIC.replace("{resource_init}", resource_init)

    for tag in tags:
        ops = tag_ops[tag]
        cls = f"{_tag_pascal(tag)}Resource"
        out += f"// ── {_tag_pascal(tag)} Resource\n\n"
        out += f"class {cls} {{\n"
        out += f"  constructor(client) {{ this._c = client; }}\n"

        for op in ops:
            op_id = op["op_id"]
            params = op["params"]
            query_params = op["query_params"]
            has_body = op["has_body"]
            is_stream = op["is_stream"]
            http = op["http"]
            path = op["path"]

            js_method = _op_camel(op_id)
            js_params = list(params)
            if has_body:
                js_params.append("data")
            if query_params:
                js_params.append("query")
            sig = ", ".join(js_params)

            path_expr = f"`{_js_path(path)}`" if params else f'"{path}"'
            body_arg = "data" if has_body else "undefined"
            query_arg = "query" if query_params else "undefined"
            call = "_stream" if is_stream else "_request"
            keyword = "async *" if is_stream else "async "
            invoke = "yield* " if is_stream else "return "

            if has_body or query_params:
                out += f'\n  {keyword}{js_method}({sig}) {{\n'
                out += f'    {invoke}this._c.{call}("{http}", {path_expr}, {body_arg}, {query_arg});\n'
                out += "  }\n"
            else:
                out += f'\n  {keyword}{js_method}({sig}) {{\n'
                out += f'    {invoke}this._c.{call}("{http}", {path_expr});\n'
                out += "  }\n"

        out += "}\n\n"

    out += "module.exports = { LibreFang, LibreFangError };\n"
    return out


# ── Go generator ──────────────────────────────────────────────────────────────

_GO_STATIC = '''\
/*
LibreFang Go SDK — AUTO-GENERATED from openapi.json.
Do not edit manually. Run: python3 scripts/codegen-sdks.py
*/
package librefang

import (
\t"bufio"
\t"bytes"
\t"encoding/json"
\t"fmt"
\t"io"
\t"net/http"
\t"net/url"
\t"strings"
)

// LibreFangError represents an API error.
type LibreFangError struct {
\tMessage string
\tStatus  int
\tBody    string
}

func (e *LibreFangError) Error() string {
\treturn fmt.Sprintf("HTTP %d: %s", e.Status, e.Message)
}

// Client is the LibreFang REST API client.
type Client struct {
\tBaseURL string
\tHeaders map[string]string
\tHTTP    *http.Client

{resource_fields}
}

// New creates a new LibreFang client.
func New(baseURL string) *Client {
\tbaseURL = strings.TrimSuffix(baseURL, "/")
\tc := &Client{
\t\tBaseURL: baseURL,
\t\tHeaders: map[string]string{"Content-Type": "application/json"},
\t\tHTTP:    &http.Client{},
\t}
{resource_init}
\treturn c
}

func (c *Client) withQuery(path string, query map[string]string) string {
\tif len(query) == 0 {
\t\treturn path
\t}
\tvals := url.Values{}
\tfor k, v := range query {
\t\tif v == "" {
\t\t\tcontinue
\t\t}
\t\tvals.Set(k, v)
\t}
\tq := vals.Encode()
\tif q == "" {
\t\treturn path
\t}
\tif strings.Contains(path, "?") {
\t\treturn path + "&" + q
\t}
\treturn path + "?" + q
}

func (c *Client) request(method, path string, body interface{}, query map[string]string) (interface{}, error) {
\turlStr := c.BaseURL + c.withQuery(path, query)
\tvar bodyBytes []byte
\tif body != nil {
\t\tb, err := json.Marshal(body)
\t\tif err != nil {
\t\t\treturn nil, fmt.Errorf("marshal: %w", err)
\t\t}
\t\tbodyBytes = b
\t}
\treq, err := http.NewRequest(method, urlStr, bytes.NewReader(bodyBytes))
\tif err != nil {
\t\treturn nil, err
\t}
\tfor k, v := range c.Headers {
\t\treq.Header.Set(k, v)
\t}
\tresp, err := c.HTTP.Do(req)
\tif err != nil {
\t\treturn nil, err
\t}
\tdefer resp.Body.Close()
\trespBody, _ := io.ReadAll(resp.Body)
\tif resp.StatusCode >= 400 {
\t\treturn nil, &LibreFangError{Message: string(respBody), Status: resp.StatusCode, Body: string(respBody)}
\t}
\tvar arr []json.RawMessage
\tif err := json.Unmarshal(respBody, &arr); err == nil {
\t\treturn arr, nil
\t}
\tvar result map[string]interface{}
\tif err := json.Unmarshal(respBody, &result); err != nil {
\t\treturn string(respBody), nil
\t}
\treturn result, nil
}

func (c *Client) stream(method, path string, body interface{}, query map[string]string) <-chan map[string]interface{} {
\tch := make(chan map[string]interface{})
\tgo func() {
\t\tdefer close(ch)
\t\turlStr := c.BaseURL + c.withQuery(path, query)
\t\tvar bodyBytes []byte
\t\tif body != nil {
\t\t\tb, err := json.Marshal(body)
\t\t\tif err != nil {
\t\t\t\tch <- map[string]interface{}{"error": fmt.Sprintf("marshal: %v", err), "status": 0}
\t\t\t\treturn
\t\t\t}
\t\t\tbodyBytes = b
\t\t}
\t\treq, err := http.NewRequest(method, urlStr, bytes.NewReader(bodyBytes))
\t\tif err != nil {
\t\t\tch <- map[string]interface{}{"error": fmt.Sprintf("new request: %v", err), "status": 0}
\t\t\treturn
\t\t}
\t\tfor k, v := range c.Headers {
\t\t\treq.Header.Set(k, v)
\t\t}
\t\treq.Header.Set("Accept", "text/event-stream")
\t\tresp, err := c.HTTP.Do(req)
\t\tif err != nil {
\t\t\tch <- map[string]interface{}{"error": err.Error(), "status": 0}
\t\t\treturn
\t\t}
\t\tdefer resp.Body.Close()
\t\tif resp.StatusCode >= 400 {
\t\t\tbody, _ := io.ReadAll(resp.Body)
\t\t\tch <- map[string]interface{}{
\t\t\t\t"error":  fmt.Sprintf("HTTP %d: %s", resp.StatusCode, string(body)),
\t\t\t\t"status": resp.StatusCode,
\t\t\t}
\t\t\treturn
\t\t}
\t\t// Accumulate partial lines across reads; SSE events can span chunks.
\t\t// bufio.Reader grows its internal buffer without bound on unterminated
\t\t// input; a limited reader plus explicit size checks cap memory use.
\t\tconst maxSSELine = 8 * 1024 * 1024
\t\treader := bufio.NewReaderSize(resp.Body, 64*1024)
\t\tfor {
\t\t\tline, err := reader.ReadString('\\n')
\t\t\tif len(line) > maxSSELine {
\t\t\t\tch <- map[string]interface{}{
\t\t\t\t\t"error":  fmt.Sprintf("SSE line exceeded %d bytes", maxSSELine),
\t\t\t\t\t"status": 0,
\t\t\t\t}
\t\t\t\treturn
\t\t\t}
\t\t\tif line != "" {
\t\t\t\ttrimmed := strings.TrimSpace(line)
\t\t\t\tif strings.HasPrefix(trimmed, "data: ") {
\t\t\t\t\tdata := strings.TrimPrefix(trimmed, "data: ")
\t\t\t\t\tif data == "[DONE]" {
\t\t\t\t\t\treturn
\t\t\t\t\t}
\t\t\t\t\tvar event map[string]interface{}
\t\t\t\t\tif jerr := json.Unmarshal([]byte(data), &event); jerr != nil {
\t\t\t\t\t\tch <- map[string]interface{}{"raw": data}
\t\t\t\t\t} else {
\t\t\t\t\t\tch <- event
\t\t\t\t\t}
\t\t\t\t}
\t\t\t}
\t\t\tif err != nil {
\t\t\t\treturn
\t\t\t}
\t\t}
\t}()
\treturn ch
}

// ToMap converts an interface{} to map[string]interface{}.
func ToMap(v interface{}) map[string]interface{} {
\tif m, ok := v.(map[string]interface{}); ok {
\t\treturn m
\t}
\treturn map[string]interface{}{}
}

// ToSlice converts an interface{} to []map[string]interface{}.
func ToSlice(v interface{}) []map[string]interface{} {
\tswitch t := v.(type) {
\tcase []json.RawMessage:
\t\tout := make([]map[string]interface{}, len(t))
\t\tfor i, raw := range t {
\t\t\tjson.Unmarshal(raw, &out[i])
\t\t}
\t\treturn out
\tcase []interface{}:
\t\tout := make([]map[string]interface{}, len(t))
\t\tfor i, a := range t {
\t\t\tif m, ok := a.(map[string]interface{}); ok {
\t\t\t\tout[i] = m
\t\t\t}
\t\t}
\t\treturn out
\t}
\treturn nil
}

'''


def gen_go(tag_ops: dict) -> str:
    tags = sorted(tag_ops)

    field_lines = []
    init_lines = []
    for tag in tags:
        attr = _tag_pascal(tag)
        cls = f"{_tag_pascal(tag)}Resource"
        field_lines.append(f"\t{attr} *{cls}")
        init_lines.append(f"\tc.{attr} = &{cls}{{client: c}}")

    resource_fields = "\n".join(field_lines)
    resource_init = "\n".join(f"\t{l}" for l in init_lines)

    out = _GO_STATIC.replace("{resource_fields}", resource_fields).replace("{resource_init}", resource_init)

    for tag in tags:
        ops = tag_ops[tag]
        cls = f"{_tag_pascal(tag)}Resource"
        out += f"// ── {_tag_pascal(tag)} Resource\n\n"
        out += f"type {cls} struct{{ client *Client }}\n\n"

        for op in ops:
            op_id = op["op_id"]
            params = op["params"]
            query_params = op["query_params"]
            has_body = op["has_body"]
            is_stream = op["is_stream"]
            http = op["http"]
            path = op["path"]

            go_method = _op_pascal(op_id)
            go_params = [f"{p} string" for p in params]
            if has_body:
                go_params.append("data map[string]interface{}")
            if query_params:
                go_params.append("query map[string]string")
            sig_args = ", ".join(go_params)

            go_path_fmt_str = _go_path(path)
            fmt_args = "".join(f", {p}" for p in params)
            path_expr = f'fmt.Sprintf("{go_path_fmt_str}"{fmt_args})' if params else f'"{path}"'
            body_arg = "data" if has_body else "nil"
            query_arg = "query" if query_params else "nil"

            if is_stream:
                out += f"func (r *{cls}) {go_method}({sig_args}) <-chan map[string]interface{{}} {{\n"
                out += f'\treturn r.client.stream("{http}", {path_expr}, {body_arg}, {query_arg})\n'
                out += "}\n\n"
            else:
                out += f"func (r *{cls}) {go_method}({sig_args}) (interface{{}}, error) {{\n"
                out += f'\treturn r.client.request("{http}", {path_expr}, {body_arg}, {query_arg})\n'
                out += "}\n\n"

    return out


# ── Rust generator ────────────────────────────────────────────────────────────

_RUST_LIB_HEADER = """\
//! LibreFang Rust SDK — AUTO-GENERATED from openapi.json.
//! Do not edit manually. Run: python3 scripts/codegen-sdks.py
//!
//! # Usage
//!
//! ```rust,no_run
//! use librefang::LibreFang;
//!
//! #[tokio::main(flavor = "current_thread")]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let client = LibreFang::new("http://localhost:4545");
//!     let health = client.system.health().await?;
//!     println!("{:?}", health);
//!     Ok(())
//! }
//! ```

use futures::StreamExt;
use reqwest::Client;
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("HTTP {status}: {body}")]
    Api { status: u16, body: String },
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

fn build_url<'a>(
    client: &Client,
    base_url: &str,
    path_segments: impl IntoIterator<Item = &'a str>,
) -> Result<reqwest::Url> {
    let path_segments: Vec<&str> = path_segments.into_iter().collect();
    if let Some(segment) = path_segments
        .iter()
        .copied()
        .find(|segment| matches!(*segment, "." | ".."))
    {
        return Err(Error::Api {
            status: 0,
            body: format!("invalid path segment: {}", segment),
        });
    }
    let mut url = client.get(base_url).build()?.url().clone();
    url.set_query(None);
    url.set_fragment(None);
    let mut segments = url
        .path_segments_mut()
        .map_err(|_| Error::Api {
            status: 0,
            body: "base URL cannot contain path segments".to_string(),
        })?;
    segments.pop_if_empty();
    segments.extend(path_segments);
    drop(segments);
    Ok(url)
}

async fn do_req(
    client: &Client,
    base_url: &str,
    method: reqwest::Method,
    path_segments: &[&str],
    body: Option<Value>,
    query: &[(&str, Option<&str>)],
) -> Result<Value> {
    let url = build_url(client, base_url, path_segments.iter().copied())?;
    let req = client
        .request(method, url)
        .timeout(DEFAULT_REQUEST_TIMEOUT);
    let filtered: Vec<(&str, &str)> = query
        .iter()
        .filter_map(|(k, v)| v.map(|vv| (*k, vv)))
        .collect();
    let req = if filtered.is_empty() { req } else { req.query(&filtered) };
    let req = if let Some(b) = body { req.json(&b) } else { req };
    let res = req.send().await?;
    let status = res.status();
    let text = res.text().await?;
    if !status.is_success() {
        return Err(Error::Api { status: status.as_u16(), body: text });
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

fn do_stream(
    client: Client,
    base_url: String,
    path_segments: Vec<String>,
    method: reqwest::Method,
    body: Option<Value>,
    query: Vec<(String, Option<String>)>,
) -> tokio::sync::mpsc::Receiver<Value> {
    const STREAM_CHANNEL_CAPACITY: usize = 256;
    let (tx, rx) = tokio::sync::mpsc::channel(STREAM_CHANNEL_CAPACITY);
    tokio::spawn(async move {
        let url = match build_url(&client, &base_url, path_segments.iter().map(String::as_str)) {
            Ok(url) => url,
            Err(e) => {
                let error = match e {
                    Error::Api { status: 0, body } => body,
                    other => other.to_string(),
                };
                let _ = tx.send(serde_json::json!({
                    "error": error,
                    "status": 0,
                })).await;
                return;
            }
        };
        let req = client.request(method, url).header("Accept", "text/event-stream");
        let filtered: Vec<(String, String)> = query
            .into_iter()
            .filter_map(|(k, v)| v.map(|vv| (k, vv)))
            .collect();
        let req = if filtered.is_empty() { req } else { req.query(&filtered) };
        let req = if let Some(b) = body { req.json(&b) } else { req };
        let res = tokio::select! {
            _ = tx.closed() => return,
            result = req.send() => match result {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(serde_json::json!({
                        "error": e.to_string(),
                        "status": 0,
                    })).await;
                    return;
                }
            }
        };
        if !res.status().is_success() {
            let status = res.status().as_u16();
            let body = tokio::select! {
                _ = tx.closed() => return,
                body = res.text() => body.unwrap_or_default(),
            };
            let _ = tx.send(serde_json::json!({
                "error": format!("HTTP {}: {}", status, body),
                "status": status,
            })).await;
            return;
        }
        // Accumulate raw bytes so multi-byte UTF-8 codepoints are not split
        // by chunk boundaries (from_utf8_lossy on individual chunks corrupts
        // non-ASCII content). Split on newline, decode each complete line.
        // MAX_SSE_LINE caps memory on misbehaving streams.
        const MAX_SSE_LINE: usize = 8 * 1024 * 1024;
        let mut stream = res.bytes_stream();
        let mut buffer: Vec<u8> = Vec::new();
        loop {
            let chunk = tokio::select! {
                _ = tx.closed() => return,
                next = stream.next() => match next {
                    Some(Ok(chunk)) => chunk,
                    Some(Err(e)) => {
                        let _ = tx.send(serde_json::json!({
                            "error": format!("stream error: {}", e),
                            "status": 0,
                        })).await;
                        return;
                    }
                    None => break,
                },
            };
            buffer.extend_from_slice(&chunk);
            if buffer.len() > MAX_SSE_LINE {
                let _ = tx.send(serde_json::json!({
                    "error": format!("SSE line exceeded {} bytes", MAX_SSE_LINE),
                    "status": 0,
                })).await;
                return;
            }
            while let Some(pos) = buffer.iter().position(|&b| b == b'\\n') {
                let line_bytes: Vec<u8> = buffer.drain(..=pos).collect();
                let line = match std::str::from_utf8(&line_bytes) {
                    Ok(s) => s.trim(),
                    Err(e) => {
                        if tx.send(serde_json::json!({
                            "error": format!("invalid utf-8 in SSE line at byte {}", e.valid_up_to()),
                            "status": 0,
                        })).await.is_err() {
                            return;
                        }
                        continue;
                    }
                };
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" { return; }
                    match serde_json::from_str::<Value>(data) {
                        Ok(v) => {
                            if tx.send(v).await.is_err() { return; }
                        }
                        Err(_) => {
                            if tx.send(serde_json::json!({"raw": data})).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        }
        // A clean EOF can arrive without a trailing newline, leaving the last event in the buffer.
        // Parse it here rather than dropping it; the loop above only fires on a newline.
        if !buffer.is_empty() {
            match std::str::from_utf8(&buffer) {
                Ok(line) => {
                    if let Some(data) = line.trim().strip_prefix("data: ") {
                        if data != "[DONE]" {
                            let event = serde_json::from_str::<Value>(data)
                                .unwrap_or_else(|_| serde_json::json!({"raw": data}));
                            let _ = tx.send(event).await;
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(serde_json::json!({
                        "error": format!("invalid utf-8 in SSE line at byte {}", e.valid_up_to()),
                        "status": 0,
                    })).await;
                }
            }
        }
    });
    rx
}

"""

_RUST_OLD_MODS = ["agents", "models", "providers", "skills", "tools"]


def _rust_path_segments(path: str, *, owned: bool) -> str:
    """Render an endpoint path as safely appended URL segments."""
    rendered = []
    for segment in path.strip("/").split("/"):
        param = re.fullmatch(r"\{([^}]+)\}", segment)
        if param:
            value = _rust_safe(param.group(1))
            rendered.append(f"{value}.to_string()" if owned else value)
        else:
            literal = json.dumps(segment)
            rendered.append(f"{literal}.to_string()" if owned else literal)
    collection = f"[{', '.join(rendered)}]"
    return f"vec!{collection}" if owned else f"&{collection}"


def gen_rust(tag_ops: dict) -> str:
    tags = sorted(tag_ops)
    out = _RUST_LIB_HEADER

    # ── LibreFang struct ──
    out += "#[derive(Debug, Clone)]\npub struct LibreFang {\n"
    for tag in tags:
        attr = _tag_attr(tag)
        cls = f"{_tag_pascal(tag)}Resource"
        out += f"    pub {attr}: Arc<{cls}>,\n"
    out += "    _base_url: String,\n"
    out += "    _client: Client,\n"
    out += "}\n\n"

    out += "impl LibreFang {\n"
    out += "    pub fn new(base_url: impl Into<String>) -> Self {\n"
    out += "        let client = Client::builder()\n"
    out += "            .connect_timeout(DEFAULT_CONNECT_TIMEOUT)\n"
    out += "            .build()\n"
    out += '            .expect("failed to build HTTP client");\n'
    out += "        Self::with_client(base_url, client)\n"
    out += "    }\n\n"
    out += "    /// Creates an SDK client using a caller-configured HTTP client.\n"
    out += "    ///\n"
    out += "    /// Use this to configure authentication headers, cookies, proxies,\n"
    out += "    /// TLS, or other [`reqwest::Client`] behavior shared by all resources.\n"
    out += "    pub fn with_client(base_url: impl Into<String>, client: Client) -> Self {\n"
    out += "        let base_url = base_url.into().trim_end_matches('/').to_string();\n"
    out += "        Self {\n"
    for tag in tags:
        attr = _tag_attr(tag)
        cls = f"{_tag_pascal(tag)}Resource"
        out += f"            {attr}: Arc::new({cls}::new(base_url.clone(), client.clone())),\n"
    out += "            _base_url: base_url,\n"
    out += "            _client: client,\n"
    out += "        }\n"
    out += "    }\n"
    out += "}\n\n"

    # ── resource impls ──
    for tag in tags:
        ops = tag_ops[tag]
        cls = f"{_tag_pascal(tag)}Resource"
        out += f"// ── {_tag_pascal(tag)} ──\n\n"
        out += f"#[derive(Debug, Clone)]\npub struct {cls} {{\n"
        out += "    base_url: String,\n"
        out += "    client: Client,\n"
        out += "}\n\n"
        out += f"impl {cls} {{\n"
        out += "    fn new(base_url: String, client: Client) -> Self {\n"
        out += "        Self { base_url, client }\n"
        out += "    }\n"

        for op in ops:
            op_id = op["op_id"]
            params = op["params"]
            query_params = op["query_params"]
            has_body = op["has_body"]
            is_stream = op["is_stream"]
            http = op["http"]
            path = op["path"]

            rust_params = [f"{_rust_safe(p)}: &str" for p in params]
            if has_body:
                rust_params.append("data: Value")
            for qp in query_params:
                rust_params.append(f"{_rust_safe(qp)}: Option<&str>")
            sig = ", ".join(["&self"] + rust_params)

            method_const = f"reqwest::Method::{http}"
            body_arg = "Some(data)" if has_body else "None"

            if is_stream:
                path_arg = _rust_path_segments(path, owned=True)
                if query_params:
                    q_items = ", ".join(
                        f'("{qp}".to_string(), {_rust_safe(qp)}.map(|s| s.to_string()))'
                        for qp in query_params
                    )
                    query_arg = f"vec![{q_items}]"
                else:
                    query_arg = "Vec::new()"
                out += f"\n    pub fn {op_id}({sig}) -> tokio::sync::mpsc::Receiver<Value> {{\n"
                out += f"        do_stream(self.client.clone(), self.base_url.clone(), {path_arg}, {method_const}, {body_arg}, {query_arg})\n"
                out += "    }\n"
            else:
                path_arg = _rust_path_segments(path, owned=False)
                if query_params:
                    q_items = ", ".join(
                        f'("{qp}", {_rust_safe(qp)})' for qp in query_params
                    )
                    query_arg = f"&[{q_items}]"
                else:
                    query_arg = "&[]"
                out += f"\n    pub async fn {op_id}({sig}) -> Result<Value> {{\n"
                out += f"        do_req(&self.client, &self.base_url, {method_const}, {path_arg}, {body_arg}, {query_arg}).await\n"
                out += "    }\n"

        out += "}\n\n"

    return out


# ── main ──────────────────────────────────────────────────────────────────────

def main():
    dry_run = "--dry-run" in sys.argv

    if not OPENAPI.exists():
        print(f"ERROR: {OPENAPI} not found", file=sys.stderr)
        sys.exit(1)

    tag_ops = load_ops()
    total_ops = sum(len(v) for v in tag_ops.values())
    print(f"Loaded {total_ops} operations across {len(tag_ops)} tags")

    outputs = {
        ROOT / "sdk/python/librefang/librefang_client.py": gen_python(tag_ops),
        ROOT / "sdk/javascript/index.js": gen_js(tag_ops),
        ROOT / "sdk/go/librefang.go": gen_go(tag_ops),
        ROOT / "sdk/rust/src/lib.rs": gen_rust(tag_ops),
    }

    for path, content in outputs.items():
        if dry_run:
            print(f"\n{'='*60}\n{path}\n{'='*60}")
            print(content[:2000], "..." if len(content) > 2000 else "")
        else:
            path.write_text(content, encoding="utf-8")
            print(f"  wrote {path.relative_to(ROOT)}  ({len(content.splitlines())} lines)")

    if not dry_run:
        # Remove old hand-written per-module files superseded by generated lib.rs
        rust_src = ROOT / "sdk/rust/src"
        for mod_name in _RUST_OLD_MODS:
            old = rust_src / f"{mod_name}.rs"
            if old.exists():
                old.unlink()
                print(f"  removed {old.relative_to(ROOT)}")

        # rustfmt the generated Rust SDK so its output is byte-identical to
        # what `cargo fmt` / `rustfmt --check` expects. Without this the
        # codegen output trips the pre-commit hook on every regen.
        # Soft-fail on either missing-rustfmt or rustfmt-rejection so a
        # syntactically-broken emitter regression surfaces as a WARN with
        # the half-emitted file on disk for inspection, rather than a
        # Python traceback. The pre-commit `cargo fmt --check` hook is
        # the hard gate either way.
        rust_out = ROOT / "sdk/rust/src/lib.rs"
        if shutil.which("rustfmt"):
            result = subprocess.run(
                ["rustfmt", "--edition", "2021", str(rust_out)],
            )
            if result.returncode == 0:
                print(f"  rustfmt {rust_out.relative_to(ROOT)}")
            else:
                print(
                    f"  WARN: rustfmt exited {result.returncode}; "
                    "sdk/rust/src/lib.rs left as emitted",
                    file=sys.stderr,
                )
        else:
            print(
                "  WARN: rustfmt not on PATH; sdk/rust/src/lib.rs left unformatted",
                file=sys.stderr,
            )


if __name__ == "__main__":
    main()
