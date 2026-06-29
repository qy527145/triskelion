//! mcp2cli：把一份 MCP 当作命令行使用。传输层在 [`crate::mcp`]，此处只做
//! CLI 侧的参数转译与帮助/结果渲染。

use anyhow::{Result, bail};
use serde_json::{Map, Value};

pub use crate::mcp::{McpClient, Tool};

// ---------------------------------------------------------------------------
// 参数转译 + 帮助渲染
// ---------------------------------------------------------------------------

/// 把 `--key value --flag` 风格按 inputSchema 类型转成 JSON-RPC arguments。
pub fn build_arguments(schema: &Value, args: &[String]) -> Result<Value> {
    let props = schema.get("properties").and_then(|p| p.as_object());
    let mut map = Map::new();
    let mut i = 0;
    while i < args.len() {
        let tok = &args[i];
        let stripped = tok.strip_prefix("--").or_else(|| tok.strip_prefix('-'));
        let key = match stripped {
            Some(k) => k,
            None => bail!("非法参数 `{tok}`，应为 --key value"),
        };
        let (name, inline) = match key.split_once('=') {
            Some((k, v)) => (k.to_string(), Some(v.to_string())),
            None => (key.to_string(), None),
        };
        let ptype = props
            .and_then(|p| p.get(&name))
            .and_then(|s| s.get("type"))
            .and_then(|t| t.as_str())
            .unwrap_or("string");
        let raw = if let Some(v) = inline {
            i += 1;
            v
        } else if ptype == "boolean" {
            i += 1;
            "true".to_string()
        } else if i + 1 < args.len() {
            let v = args[i + 1].clone();
            i += 2;
            v
        } else {
            i += 1;
            "true".to_string()
        };
        map.insert(name, coerce(ptype, &raw));
    }
    Ok(Value::Object(map))
}

fn coerce(ptype: &str, raw: &str) -> Value {
    match ptype {
        "integer" | "number" => raw
            .parse::<i64>()
            .map(Value::from)
            .or_else(|_| raw.parse::<f64>().map(Value::from))
            .unwrap_or_else(|_| Value::String(raw.to_string())),
        "boolean" => Value::Bool(matches!(raw, "true" | "1" | "yes")),
        // 结构化类型：按 JSON 解析，解析失败再退回字符串。
        "array" | "object" => {
            serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
        }
        "string" => Value::String(raw.to_string()),
        // 未声明类型：值看起来像 JSON（[ 或 {）时尝试解析，否则当字符串。
        _ => {
            let t = raw.trim_start();
            if t.starts_with('[') || t.starts_with('{') {
                serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
            } else {
                Value::String(raw.to_string())
            }
        }
    }
}

pub fn overview(pkg: &str, name: &str, tools: &[Tool]) {
    println!("{name} — {} 个工具", tools.len());
    println!("用法: tsk run {pkg} <tool> [--arg value ...]\n");
    if tools.is_empty() {
        println!("(该 MCP 未暴露任何工具)");
        return;
    }
    let width = tools.iter().map(|t| t.name.len()).max().unwrap_or(0);
    println!("可用工具:");
    for t in tools {
        let desc = t.description.lines().next().unwrap_or("");
        println!("  {:<width$}  {}", t.name, desc, width = width);
    }
    println!("\n查看某工具参数: tsk run {pkg} <tool> --help");
}

pub fn tool_help(pkg: &str, tool: &Tool) {
    println!("{}: {}", tool.name, tool.description);
    println!("用法: tsk run {pkg} {} [--arg value ...]\n", tool.name);
    let required: Vec<&str> = tool
        .input_schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    match tool.input_schema.get("properties").and_then(|p| p.as_object()) {
        Some(props) if !props.is_empty() => {
            println!("参数:");
            for (k, v) in props {
                let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("string");
                let req = if required.contains(&k.as_str()) { " (必填)" } else { "" };
                let desc = v.get("description").and_then(|d| d.as_str()).unwrap_or("");
                println!("  --{k} <{ty}>{req}  {desc}");
            }
        }
        _ => println!("(无参数)"),
    }
}

/// 渲染 tools/call 结果；isError 时返回 false 供上层置非零退出码。
pub fn print_result(result: &Value) -> bool {
    if let Some(content) = result.get("content").and_then(|c| c.as_array()) {
        for item in content {
            match item.get("type").and_then(|t| t.as_str()) {
                Some("text") => {
                    if let Some(t) = item.get("text").and_then(|t| t.as_str()) {
                        println!("{t}");
                    }
                }
                _ => println!("{}", serde_json::to_string_pretty(item).unwrap_or_default()),
            }
        }
    } else {
        println!("{}", serde_json::to_string_pretty(result).unwrap_or_default());
    }
    !result.get("isError").and_then(|e| e.as_bool()).unwrap_or(false)
}
