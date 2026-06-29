//! MCP 传输层：连接一份 MCP 清单并完成 JSON-RPC 交互。
//!
//! 双侧复用：
//! - `tsk` 客户端（mcp2cli）在本地连接并把 MCP 当 CLI 使用；
//! - `triskelion` Hub 作为网关，服务端连接并代调用（Web 测试调用）。
//!
//! local/stdio 拉起子进程，remote/streamable|sse 走阻塞 HTTP JSON-RPC。

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use crate::shared::{McpManifest, Protocol, Runtime};

const PROTOCOL_VERSION: &str = "2025-06-18";

#[derive(Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

pub enum McpClient {
    Stdio(StdioClient),
    Http(HttpClient),
}

impl McpClient {
    /// 按拓扑连接并完成 MCP initialize 握手。
    pub fn connect(m: &McpManifest) -> Result<Self> {
        match m.runtime {
            Runtime::Local => {
                let cmd = m
                    .command
                    .as_ref()
                    .ok_or_else(|| anyhow!("local 运行时缺少 command"))?;
                let mut c = StdioClient::spawn(cmd, &m.env)?;
                c.initialize()?;
                Ok(McpClient::Stdio(c))
            }
            Runtime::Remote => {
                let url = m.url.as_ref().ok_or_else(|| anyhow!("remote 运行时缺少 url"))?;
                let mut c = HttpClient::new(url, &m.headers, m.protocol);
                c.initialize()?;
                Ok(McpClient::Http(c))
            }
        }
    }

    pub fn list_tools(&mut self) -> Result<Vec<Tool>> {
        let result = self.rpc("tools/list", json!({}))?;
        let arr = result
            .get("tools")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(arr
            .into_iter()
            .map(|t| Tool {
                name: t.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                description: t
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                input_schema: t.get("inputSchema").cloned().unwrap_or(json!({})),
            })
            .collect())
    }

    pub fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value> {
        self.rpc("tools/call", json!({"name": name, "arguments": arguments}))
    }

    fn rpc(&mut self, method: &str, params: Value) -> Result<Value> {
        match self {
            McpClient::Stdio(c) => c.rpc(method, params),
            McpClient::Http(c) => c.rpc(method, params),
        }
    }
}

// ---------------------------------------------------------------------------
// stdio 子进程传输
// ---------------------------------------------------------------------------

pub struct StdioClient {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    next_id: i64,
}

impl StdioClient {
    fn spawn(command: &str, env: &std::collections::BTreeMap<String, String>) -> Result<Self> {
        let parts: Vec<&str> = command.split_whitespace().collect();
        let (prog, args) = parts.split_first().ok_or_else(|| anyhow!("command 为空"))?;
        let mut cmd = Command::new(prog);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd
            .spawn()
            .with_context(|| format!("拉起本地 MCP 进程: {command}"))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("无法获取 stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("无法获取 stdout"))?;
        Ok(StdioClient {
            child,
            stdin,
            reader: BufReader::new(stdout),
            next_id: 1,
        })
    }

    fn initialize(&mut self) -> Result<()> {
        self.rpc(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "triskelion", "version": "0.1.0"}
            }),
        )?;
        self.notify("notifications/initialized", json!({}))?;
        Ok(())
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let msg = json!({"jsonrpc": "2.0", "method": method, "params": params});
        writeln!(self.stdin, "{}", serde_json::to_string(&msg)?)?;
        self.stdin.flush()?;
        Ok(())
    }

    fn rpc(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{}", serde_json::to_string(&msg)?)?;
        self.stdin.flush()?;
        loop {
            let mut line = String::new();
            let n = self.reader.read_line(&mut line)?;
            if n == 0 {
                bail!("本地 MCP 进程在响应 {method} 前关闭了 stdout");
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let v: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue, // 跳过非 JSON 日志
            };
            if v.get("id").and_then(|x| x.as_i64()) == Some(id) {
                if let Some(err) = v.get("error") {
                    bail!("MCP 错误: {err}");
                }
                return Ok(v.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }
}

impl Drop for StdioClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------
// HTTP 传输（streamable / sse）
// ---------------------------------------------------------------------------

pub struct HttpClient {
    http: reqwest::blocking::Client,
    url: String,
    headers: Vec<(String, String)>,
    session_id: Option<String>,
    next_id: i64,
}

impl HttpClient {
    fn new(url: &str, headers: &std::collections::BTreeMap<String, String>, _p: Protocol) -> Self {
        HttpClient {
            http: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("构建 HTTP 客户端"),
            url: url.to_string(),
            headers: headers.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            session_id: None,
            next_id: 1,
        }
    }

    fn initialize(&mut self) -> Result<()> {
        self.rpc(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "triskelion", "version": "0.1.0"}
            }),
        )?;
        let _ = self.post("notifications/initialized", json!({}), false);
        Ok(())
    }

    fn rpc(&mut self, method: &str, params: Value) -> Result<Value> {
        self.post(method, params, true)
    }

    fn post(&mut self, method: &str, params: Value, want_result: bool) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let body = if want_result {
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
        } else {
            json!({"jsonrpc": "2.0", "method": method, "params": params})
        };
        let mut rb = self
            .http
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json");
        for (k, v) in &self.headers {
            rb = rb.header(k, v);
        }
        if let Some(sid) = &self.session_id {
            rb = rb.header("Mcp-Session-Id", sid);
        }
        let resp = rb
            .json(&body)
            .send()
            .with_context(|| format!("请求远程 MCP {} 失败", self.url))?;
        if let Some(h) = resp.headers().get("mcp-session-id")
            && let Ok(s) = h.to_str()
        {
            self.session_id = Some(s.to_string());
        }
        let status = resp.status();
        let ctype = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let text = resp.text().unwrap_or_default();
        if !status.is_success() {
            bail!("远程 MCP HTTP {}: {}", status.as_u16(), truncate(&text, 400));
        }
        if !want_result {
            return Ok(Value::Null);
        }
        let msg = if ctype.contains("text/event-stream") {
            parse_sse(&text, id)?
        } else {
            serde_json::from_str(&text)
                .with_context(|| format!("远程 MCP 响应非 JSON: {}", truncate(&text, 400)))?
        };
        if let Some(err) = msg.get("error") {
            bail!("MCP 错误: {err}");
        }
        Ok(msg.get("result").cloned().unwrap_or(Value::Null))
    }
}

/// 从 SSE 文本里挑出 id 匹配的 JSON-RPC 消息。
fn parse_sse(text: &str, id: i64) -> Result<Value> {
    let mut last = None;
    for line in text.lines() {
        let line = line.trim_start();
        if let Some(data) = line.strip_prefix("data:")
            && let Ok(v) = serde_json::from_str::<Value>(data.trim())
        {
            if v.get("id").and_then(|x| x.as_i64()) == Some(id) {
                return Ok(v);
            }
            last = Some(v);
        }
    }
    last.ok_or_else(|| anyhow!("SSE 流中未找到有效 JSON-RPC 响应"))
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n])
    }
}
