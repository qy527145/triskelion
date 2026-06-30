//! 共享层：MCP 包清单（与 `mcp.json` 对齐）、Hub 开放 API 的 wire 类型、
//! JSONC 解析与渐进式凭据缝合所需的占位符插值。client / server 双侧复用。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// 运行时拓扑（参见 design.md §2）。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    /// 本地 std::process::Command 拉起 stdio 进程。
    Local,
    /// 远程 SSE/HTTP 集群，Hub 仅做反向代理。
    Remote,
}

impl Runtime {
    pub fn as_str(self) -> &'static str {
        match self {
            Runtime::Local => "local",
            Runtime::Remote => "remote",
        }
    }
}

/// 传输协议。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Stdio,
    Sse,
    Streamable,
}

impl Protocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Protocol::Stdio => "stdio",
            Protocol::Sse => "sse",
            Protocol::Streamable => "streamable",
        }
    }
}

/// MCP 包清单。字段与仓库根 `mcp.json` 对齐：local/stdio 用 `command`+`env`，
/// remote/sse|streamable 用 `url`+`headers`。值中以 `{VAR}` 形式声明所需凭据。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct McpManifest {
    #[serde(default = "default_resource_type")]
    pub resource_type: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_version")]
    pub version: String,
    pub runtime: Runtime,
    pub protocol: Protocol,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

fn default_resource_type() -> String {
    "mcp".into()
}
fn default_version() -> String {
    "0.1.0".into()
}

impl McpManifest {
    /// 收集清单里所有 `{VAR}` 占位符（去重）——即该包声明需绑定的凭据。
    pub fn required_vars(&self) -> Vec<String> {
        let mut found = Vec::new();
        let mut sink = |s: &str| {
            for v in find_placeholders(s) {
                if !found.contains(&v) {
                    found.push(v);
                }
            }
        };
        if let Some(u) = &self.url {
            sink(u);
        }
        if let Some(c) = &self.command {
            sink(c);
        }
        for v in self.env.values() {
            sink(v);
        }
        for v in self.headers.values() {
            sink(v);
        }
        found
    }
}

/// 在字符串内查找 `{IDENT}` 占位符，IDENT 由 `[A-Za-z0-9_]` 组成。
pub fn find_placeholders(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let mut j = i + 1;
            while j < bytes.len()
                && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_')
            {
                j += 1;
            }
            if j > i + 1 && j < bytes.len() && bytes[j] == b'}' {
                out.push(s[i + 1..j].to_string());
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// 用 `vars` 替换字符串里的 `{VAR}`，返回替换后文本与缺失变量名列表。
pub fn interpolate(s: &str, vars: &BTreeMap<String, String>) -> (String, Vec<String>) {
    let mut missing = Vec::new();
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let mut j = i + 1;
            while j < bytes.len()
                && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_')
            {
                j += 1;
            }
            if j > i + 1 && j < bytes.len() && bytes[j] == b'}' {
                let key = &s[i + 1..j];
                match vars.get(key) {
                    Some(v) => out.push_str(v),
                    None => {
                        if !missing.iter().any(|m| m == key) {
                            missing.push(key.to_string());
                        }
                        out.push_str(&s[i..=j]);
                    }
                }
                i = j + 1;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    (out, missing)
}

/// 渐进式凭据缝合：把全部 `{VAR}` 替换为用户密钥，返回结果与缺失列表。
pub fn stitch(manifest: &McpManifest, vars: &BTreeMap<String, String>) -> (McpManifest, Vec<String>) {
    let mut m = manifest.clone();
    let mut missing: Vec<String> = Vec::new();
    let mut merge = |miss: Vec<String>| {
        for v in miss {
            if !missing.contains(&v) {
                missing.push(v);
            }
        }
    };
    if let Some(u) = &m.url {
        let (s, miss) = interpolate(u, vars);
        m.url = Some(s);
        merge(miss);
    }
    if let Some(c) = &m.command {
        let (s, miss) = interpolate(c, vars);
        m.command = Some(s);
        merge(miss);
    }
    for v in m.env.values_mut() {
        let (s, miss) = interpolate(v, vars);
        *v = s;
        merge(miss);
    }
    for v in m.headers.values_mut() {
        let (s, miss) = interpolate(v, vars);
        *v = s;
        merge(miss);
    }
    (m, missing)
}

/// 去掉 JSONC 注释（`//` 与 `/* */`），保留字符串内容。mcp.json 含注释。
pub fn strip_jsonc(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                chars.next();
                while let Some(&n) = chars.peek() {
                    if n == '\n' {
                        break;
                    }
                    chars.next();
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = '\0';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
            }
            _ => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Hub 开放 API wire 类型
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
pub struct AuthReq {
    pub username: String,
    pub password: String,
}

#[derive(Serialize, Deserialize)]
pub struct AuthResp {
    pub token: String,
    pub username: String,
}

#[derive(Serialize, Deserialize)]
pub struct McpUpsertReq {
    pub manifest: McpManifest,
    #[serde(default = "default_visibility")]
    pub visibility: String,
}

/// 一个 MCP 工具的检索元信息（由客户端连接 MCP 后上报）。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ToolMeta {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// 工具入参 JSON Schema，供 Web 渲染测试表单。
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub input_schema: serde_json::Value,
}

/// 上报某 MCP 的工具清单（用于检索索引）。
#[derive(Serialize, Deserialize)]
pub struct SetToolsReq {
    pub tools: Vec<ToolMeta>,
}

/// Web/CLI 通过 Hub 网关代调用某工具。
#[derive(Serialize, Deserialize)]
pub struct CallReq {
    pub tool: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

/// `tsk run` 在本地直连 MCP 调用工具后，把调用结果**回传** Hub 作审计统计。
/// CLI 路径不经 Hub 网关，否则这些调用不会出现在管理后台的调用日志/热门工具里。
#[derive(Serialize, Deserialize)]
pub struct ReportCallReq {
    pub tool: String,
    pub ok: bool,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub ms: i64,
    /// 结果摘要（成功时的结果概要，便于审计面板展示；可空）。
    #[serde(default)]
    pub summary: String,
}

#[derive(Serialize, Deserialize)]
pub struct McpRenameReq {
    pub new_name: String,
}

fn default_visibility() -> String {
    "private".into()
}

#[derive(Serialize, Deserialize)]
pub struct McpInfo {
    pub owner: String,
    pub name: String,
    pub visibility: String,
    pub version: String,
    pub manifest: McpManifest,
    #[serde(default)]
    pub tools: Vec<ToolMeta>,
    /// 受管标签名（管理后台分配，如「官方」「社区」），用于市场标注与筛选。
    #[serde(default)]
    pub labels: Vec<String>,
    pub updated_at: String,
}

#[derive(Serialize, Deserialize)]
pub struct SecretSetReq {
    pub key: String,
    pub value: String,
}

#[derive(Serialize, Deserialize)]
pub struct SecretInfo {
    pub key: String,
    pub updated_at: String,
}

/// `tsk run` 的解析响应：Hub 返回**原始** manifest（含 `{VAR}` 占位符）、该包声明的
/// 全部变量（`required`），以及调用者**线上**已设置的相关变量值（`vars`，仅在已登录时非空）。
/// 客户端据此用「本地变量覆盖线上变量」的优先级在本地完成凭据缝合。
#[derive(Serialize, Deserialize)]
pub struct ResolveResp {
    pub manifest: McpManifest,
    /// 该包依赖的全部变量名（无论是否已设置）。
    #[serde(default)]
    pub required: Vec<String>,
    /// 调用者线上已设置且被该包引用的变量（名→值）。未登录时为空。
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
pub struct ErrorResp {
    pub error: String,
}

// ---------------------------------------------------------------------------
// 技能市场（Skill marketplace）wire 类型
//
// 万物皆 Skill：`category` 仅是逻辑分类标签（skill / kb / toolchain），底层共用
// 同一数据结构。技能包可能是一个很大的文件夹（必须含 SKILL.md），由 `tsk build`
// 打包成 tar.zst 压缩体（zstd）；服务端只接收元数据 + SKILL.md 文本，庞大的数据体以压缩包
// 形式按 sha256 内容寻址承载。
// ---------------------------------------------------------------------------

/// 已知的逻辑分类。仅用于校验/默认，存储与检索均按字符串处理（保持可扩展）。
pub const SKILL_CATEGORIES: [&str; 3] = ["skill", "kb", "toolchain"];

/// 技能包清单，对应本地 `tsk-skill.json`。SKILL.md 不在此处，单独承载。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SkillManifest {
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    /// 逻辑分类：skill（技能）/ kb（知识库）/ toolchain（工具链）。
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// 该技能依赖的底层 MCP（`owner/name`），运行时用 `tsk run` 包装调用。
    #[serde(default)]
    pub mcp_dependencies: Vec<String>,
    /// 倾向优先使用的工具（自由文本提示，如 `github/create_issue`）。
    #[serde(default)]
    pub preferred_tools: Vec<String>,
}

fn default_category() -> String {
    "skill".into()
}

impl SkillManifest {
    /// 用目录名兜底生成一个最小清单。
    pub fn minimal(name: impl Into<String>) -> Self {
        SkillManifest {
            name: name.into(),
            version: default_version(),
            category: default_category(),
            description: String::new(),
            tags: Vec::new(),
            mcp_dependencies: Vec::new(),
            preferred_tools: Vec::new(),
        }
    }
}

/// 发布/更新一个技能的元数据（不含压缩体本身，压缩体走 archive 上传接口）。
#[derive(Serialize, Deserialize)]
pub struct SkillUpsertReq {
    pub manifest: SkillManifest,
    #[serde(default = "default_visibility")]
    pub visibility: String,
    /// SKILL.md 全文（服务端持有的「基础信息」）。
    #[serde(default)]
    pub skill_md: String,
    /// 压缩体 sha256（十六进制，空表示纯文本裸说明书包，无数据体）。
    #[serde(default)]
    pub archive_sha256: String,
    #[serde(default)]
    pub archive_size: u64,
}

/// 技能元信息（市场/详情返回）。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SkillInfo {
    pub owner: String,
    pub name: String,
    pub category: String,
    pub visibility: String,
    pub version: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub mcp_dependencies: Vec<String>,
    #[serde(default)]
    pub preferred_tools: Vec<String>,
    #[serde(default)]
    pub skill_md: String,
    #[serde(default)]
    pub archive_sha256: String,
    #[serde(default)]
    pub archive_size: u64,
    /// 受管标签名（管理后台分配，如「官方」「社区」），用于市场标注与筛选。
    #[serde(default)]
    pub labels: Vec<String>,
    pub updated_at: String,
}

#[derive(Serialize, Deserialize)]
pub struct SkillRenameReq {
    pub new_name: String,
}
