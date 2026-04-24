#!/usr/bin/env python3
"""
codegen-sdks: auto-generates Python, JS, and Go SDKs from openapi.json.

Usage:
    python3 scripts/codegen-sdks.py           # regenerate all SDKs
    python3 scripts/codegen-sdks.py --dry-run # print diffs, don't write
"""
import json
import re
import sys
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


# ── load operations ───────────────────────────────────────────────────────────

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
            if not op_id or op_id in seen:
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
from typing import Any, Dict, Generator, Optional
from urllib.request import urlopen, Request
from urllib.error import HTTPError
from urllib.parse import urlencode


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

    def __init__(self, base_url: str, headers: Optional[Dict[str, str]] = None):
        self.base_url = base_url.rstrip("/")
        self._headers = {"Content-Type": "application/json"}
        if headers:
            self._headers.update(headers)
{resource_init}
    def _request(self, method: str, path: str, body: Any = None) -> Any:
        url = self.base_url + path
        data = json.dumps(body).encode() if body is not None else None
        req = Request(url, data=data, headers=self._headers, method=method)
        try:
            with urlopen(req) as resp:
                ct = resp.headers.get("content-type", "")
                text = resp.read().decode()
                if "application/json" in ct:
                    return json.loads(text)
                return text
        except HTTPError as e:
            body_text = e.read().decode() if e.fp else ""
            raise LibreFangError(f"HTTP {e.code}: {body_text}", e.code, body_text) from e

    def _stream(self, method: str, path: str, body: Any = None) -> Generator[Dict, None, None]:
        """SSE streaming — yields parsed JSON events."""
        url = self.base_url + path
        data = json.dumps(body).encode() if body is not None else None
        headers = dict(self._headers)
        headers["Accept"] = "text/event-stream"
        req = Request(url, data=data, headers=headers, method=method)
        try:
            resp = urlopen(req)
        except HTTPError as e:
            body_text = e.read().decode() if e.fp else ""
            raise LibreFangError(f"HTTP {e.code}: {body_text}", e.code, body_text) from e

        buffer = ""
        while True:
            chunk = resp.read(4096)
            if not chunk:
                break
            buffer += chunk.decode()
            lines = buffer.split("\\n")
            buffer = lines.pop()
            for line in lines:
                line = line.strip()
                if line.startswith("data: "):
                    data_str = line[6:]
                    if data_str == "[DONE]":
                        return
                    try:
                        yield json.loads(data_str)
                    except json.JSONDecodeError:
                        yield {"raw": data_str}
        resp.close()

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
            has_body = op["has_body"]
            is_stream = op["is_stream"]
            http = op["http"]
            path = op["path"]

            sig_parts = ["self"] + [f"{p}: str" for p in params]
            if has_body:
                sig_parts.append("**data")

            sig = ", ".join(sig_parts)
            path_expr = f'f"{_py_path(path)}"' if params else f'"{path}"'

            if is_stream:
                ret_type = " -> Generator[Dict, None, None]"
            else:
                ret_type = ""

            out += f"\n    def {op_id}({sig}){ret_type}:\n"
            if is_stream:
                body_arg = ", data" if has_body else ""
                out += f'        return self._c._stream("{http}", {path_expr}{body_arg})\n'
            elif has_body:
                out += f'        return self._c._request("{http}", {path_expr}, data)\n'
            else:
                out += f'        return self._c._request("{http}", {path_expr})\n'

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

  async _request(method, path, body) {
    const url = this.baseUrl + path;
    const opts = { method, headers: this._headers };
    if (body !== undefined) opts.body = JSON.stringify(body);
    const res = await fetch(url, opts);
    const text = await res.text();
    if (!res.ok) throw new LibreFangError(`HTTP ${res.status}: ${text}`, res.status, text);
    const ct = res.headers.get("content-type") || "";
    return ct.includes("application/json") ? JSON.parse(text) : text;
  }

  async *_stream(method, path, body) {
    const url = this.baseUrl + path;
    const headers = Object.assign({}, this._headers, { Accept: "text/event-stream" });
    const opts = { method, headers };
    if (body !== undefined) opts.body = JSON.stringify(body);
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
            has_body = op["has_body"]
            is_stream = op["is_stream"]
            http = op["http"]
            path = op["path"]

            js_method = _op_camel(op_id)
            js_params = list(params)
            if has_body:
                js_params.append("data")
            sig = ", ".join(js_params)

            path_expr = f"`{_js_path(path)}`" if params else f'"{path}"'

            if is_stream:
                body_arg = ", data" if has_body else ", undefined"
                out += f'\n  async *{js_method}({sig}) {{\n'
                out += f'    yield* this._c._stream("{http}", {path_expr}{body_arg});\n'
                out += "  }\n"
            elif has_body:
                out += f'\n  async {js_method}({sig}) {{\n'
                out += f'    return this._c._request("{http}", {path_expr}, data);\n'
                out += "  }\n"
            else:
                out += f'\n  async {js_method}({sig}) {{\n'
                out += f'    return this._c._request("{http}", {path_expr});\n'
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
\t"bytes"
\t"encoding/json"
\t"fmt"
\t"io"
\t"net/http"
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

func (c *Client) request(method, path string, body interface{}) (interface{}, error) {
\turl := c.BaseURL + path
\tvar bodyBytes []byte
\tif body != nil {
\t\tb, err := json.Marshal(body)
\t\tif err != nil {
\t\t\treturn nil, fmt.Errorf("marshal: %w", err)
\t\t}
\t\tbodyBytes = b
\t}
\treq, err := http.NewRequest(method, url, bytes.NewReader(bodyBytes))
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

func (c *Client) stream(method, path string, body interface{}) <-chan map[string]interface{} {
\tch := make(chan map[string]interface{})
\tgo func() {
\t\tdefer close(ch)
\t\turl := c.BaseURL + path
\t\tvar bodyBytes []byte
\t\tif body != nil {
\t\t\tb, _ := json.Marshal(body)
\t\t\tbodyBytes = b
\t\t}
\t\treq, _ := http.NewRequest(method, url, bytes.NewReader(bodyBytes))
\t\tfor k, v := range c.Headers {
\t\t\treq.Header.Set(k, v)
\t\t}
\t\treq.Header.Set("Accept", "text/event-stream")
\t\tresp, err := c.HTTP.Do(req)
\t\tif err != nil {
\t\t\tch <- map[string]interface{}{"error": err.Error()}
\t\t\treturn
\t\t}
\t\tdefer resp.Body.Close()
\t\tif resp.StatusCode >= 400 {
\t\t\tbody, _ := io.ReadAll(resp.Body)
\t\t\tch <- map[string]interface{}{"error": fmt.Sprintf("HTTP %d: %s", resp.StatusCode, string(body))}
\t\t\treturn
\t\t}
\t\tbuf := make([]byte, 4096)
\t\tfor {
\t\t\tn, err := resp.Body.Read(buf)
\t\t\tif n > 0 {
\t\t\t\tfor _, line := range strings.Split(string(buf[:n]), "\\n") {
\t\t\t\t\tline = strings.TrimSpace(line)
\t\t\t\t\tif !strings.HasPrefix(line, "data: ") {
\t\t\t\t\t\tcontinue
\t\t\t\t\t}
\t\t\t\t\tdata := strings.TrimPrefix(line, "data: ")
\t\t\t\t\tif data == "[DONE]" {
\t\t\t\t\t\treturn
\t\t\t\t\t}
\t\t\t\t\tvar event map[string]interface{}
\t\t\t\t\tif err := json.Unmarshal([]byte(data), &event); err != nil {
\t\t\t\t\t\tch <- map[string]interface{}{"raw": data}
\t\t\t\t\t} else {
\t\t\t\t\t\tch <- event
\t\t\t\t\t}
\t\t\t\t}
\t\t\t}
\t\t\tif err != nil {
\t\t\t\tbreak
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
            has_body = op["has_body"]
            is_stream = op["is_stream"]
            http = op["http"]
            path = op["path"]

            go_method = _op_pascal(op_id)
            go_params = [f"{p} string" for p in params]
            go_path_fmt_str = _go_path(path)

            if is_stream:
                if has_body:
                    go_params.append("data map[string]interface{}")
                sig_args = ", ".join(go_params)
                out += f"func (r *{cls}) {go_method}({sig_args}) <-chan map[string]interface{{}} {{\n"
                fmt_args = "".join(f", {p}" for p in params)
                path_expr = f'fmt.Sprintf("{go_path_fmt_str}"{fmt_args})' if params else f'"{path}"'
                body_arg = "data" if has_body else "nil"
                out += f'\treturn r.client.stream("{http}", {path_expr}, {body_arg})\n'
                out += "}\n\n"
            else:
                if has_body:
                    go_params.append("data map[string]interface{}")
                sig_args = ", ".join(go_params)
                out += f"func (r *{cls}) {go_method}({sig_args}) (interface{{}}, error) {{\n"
                fmt_args = "".join(f", {p}" for p in params)
                path_expr = f'fmt.Sprintf("{go_path_fmt_str}"{fmt_args})' if params else f'"{path}"'
                body_arg = "data" if has_body else "nil"
                out += f'\treturn r.client.request("{http}", {path_expr}, {body_arg})\n'
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
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let client = LibreFang::new("http://localhost:4545");
//!     let agents = client.agents.list_agents().await?;
//!     println!("{:?}", agents);
//!     Ok(())
//! }
//! ```

use futures::StreamExt;
use reqwest::Client;
use serde_json::Value;
use std::sync::Arc;
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

async fn do_req(
    client: &Client,
    base_url: &str,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    let url = format!("{}{}", base_url, path);
    let req = client.request(method, &url);
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
    path: String,
    method: reqwest::Method,
    body: Option<Value>,
) -> tokio::sync::mpsc::UnboundedReceiver<Value> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        let url = format!("{}{}", base_url, path);
        let req = client.request(method, &url).header("Accept", "text/event-stream");
        let req = if let Some(b) = body { req.json(&b) } else { req };
        let res = match req.send().await {
            Ok(r) if r.status().is_success() => r,
            _ => return,
        };
        let mut stream = res.bytes_stream();
        let mut buffer = String::new();
        while let Some(Ok(chunk)) = stream.next().await {
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = buffer.find('\\n') {
                let line = buffer[..pos].trim().to_string();
                buffer = buffer[pos + 1..].to_string();
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" { return; }
                    if let Ok(v) = serde_json::from_str(data) {
                        let _ = tx.send(v);
                    }
                }
            }
        }
    });
    rx
}

"""

_RUST_OLD_MODS = ["agents", "models", "providers", "skills", "tools"]


def _rust_path_fmt(path: str) -> str:
    """'/api/agents/{id}' → '/api/agents/{}' (Rust format! style)"""
    return re.sub(r"\{[^}]+\}", "{}", path)


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
    out += "        let base_url = base_url.into().trim_end_matches('/').to_string();\n"
    out += "        let client = Client::new();\n"
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
            has_body = op["has_body"]
            is_stream = op["is_stream"]
            http = op["http"]
            path = op["path"]

            rust_params = [f"{p}: &str" for p in params]
            if has_body:
                rust_params.append("data: Value")
            sig = ", ".join(["&self"] + rust_params)

            fmt_path = _rust_path_fmt(path)
            fmt_args = "".join(f", {p}" for p in params)
            path_expr = (
                f'format!("{fmt_path}"{fmt_args})'
                if params
                else f'"{path}".to_string()'
            )

            method_const = f"reqwest::Method::{http}"
            body_arg = "Some(data)" if has_body else "None"

            if is_stream:
                out += f"\n    pub fn {op_id}({sig}) -> tokio::sync::mpsc::UnboundedReceiver<Value> {{\n"
                out += f"        do_stream(self.client.clone(), self.base_url.clone(), {path_expr}, {method_const}, {body_arg})\n"
                out += "    }\n"
            else:
                out += f"\n    pub async fn {op_id}({sig}) -> Result<Value> {{\n"
                out += f"        do_req(&self.client, &self.base_url, {method_const}, &{path_expr}, {body_arg}).await\n"
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


if __name__ == "__main__":
    main()
