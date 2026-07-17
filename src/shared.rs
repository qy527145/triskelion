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
    /// 点赞 / 收藏总数。
    #[serde(default)]
    pub likes: i64,
    #[serde(default)]
    pub favorites: i64,
    /// 当前查看者是否已点赞 / 收藏（未登录恒 false）。
    #[serde(default)]
    pub liked: bool,
    #[serde(default)]
    pub favorited: bool,
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
// 万物皆 Skill：`category` 仅是逻辑分类标签（skill / kb / toolchain / agent），底层共用
// 同一数据结构。技能包可能是一个很大的文件夹（必须含说明书：skill/kb/toolchain 为
// SKILL.md，agent 为 AGENT.md），由 `tsk build` 打包成 tar.zst 压缩体（zstd）；服务端只
// 接收元数据 + 说明书文本（统一以 skill_md 字段承载），庞大的数据体以压缩包形式按 sha256
// 内容寻址承载。
// ---------------------------------------------------------------------------

/// 已知的逻辑分类。仅用于校验/默认，存储与检索均按字符串处理（保持可扩展）。
pub const SKILL_CATEGORIES: [&str; 4] = ["skill", "kb", "toolchain", "agent"];

/// 不同分类的「能力说明书」文件名：agent 分类用 `AGENT.md`，其余一律用 `SKILL.md`。
/// 服务端始终以 `skill_md` 字段承载其全文，文件名仅用于本地打包/解包与市场展示。
pub fn doc_filename(category: &str) -> &'static str {
    if category == "agent" {
        "AGENT.md"
    } else {
        "SKILL.md"
    }
}

/// 从说明书（SKILL.md / AGENT.md）提取一句话描述：优先解析 YAML frontmatter 的
/// `description:` 字段（Anthropic 风格技能包惯用 `--- name/description ---` 头），
/// 否则退回首个有意义的正文行。`tsk build` 与 Web 端「拖入压缩包创建技能」共用此逻辑。
pub fn extract_description(md: &str) -> String {
    if let Some(d) = frontmatter_field(md, "description") {
        return clip(&d, 240);
    }
    first_meaningful_line(md)
}

/// frontmatter 顶层字段值：单行标量，或字符串列表（内联 `[a, b]` / 块列表 `- item`）。
#[derive(Debug, Clone, PartialEq)]
pub enum FmValue {
    Scalar(String),
    List(Vec<String>),
}

impl FmValue {
    /// 宽松转列表：列表原样，标量按逗号切分（`tags: a, b` 与 `tags: [a, b]` 等价）。
    pub fn into_list(self) -> Vec<String> {
        let items = match self {
            FmValue::List(v) => v,
            FmValue::Scalar(s) => s.split(',').map(str::to_string).collect(),
        };
        items
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

/// 去掉 YAML 标量外层引号与空白。
fn unquote(s: &str) -> String {
    s.trim().trim_matches('"').trim_matches('\'').trim().to_string()
}

/// 解析 `---` 围栏 YAML frontmatter 的顶层字段。说明书（SKILL.md / AGENT.md）的
/// frontmatter 即技能元数据载体，此处只实现所需子集：单行标量、内联数组、块列表。
/// 所有值一律按字符串处理，刻意规避 YAML 隐式类型的坑（`version: 1.10` 不会变成浮点
/// 1.1）；缩进行（嵌套结构/多行值）与未知形态一律跳过，兼容第三方生态的附加字段
/// （如 Claude Code 的 `allowed-tools`）。
pub fn parse_frontmatter(md: &str) -> Vec<(String, FmValue)> {
    let mut out: Vec<(String, FmValue)> = Vec::new();
    let mut lines = md.lines().peekable();
    if lines.next().map(str::trim) != Some("---") {
        return out;
    }
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed == "---" || trimmed == "..." {
            break;
        }
        // 顶层字段必须无缩进；缩进行属于上一字段的嵌套内容，跳过。
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || line.starts_with(' ')
            || line.starts_with('\t')
        {
            continue;
        }
        let Some((key, rest)) = line.split_once(':') else { continue };
        let (key, rest) = (key.trim().to_string(), rest.trim());
        let value = if rest.is_empty() {
            // 可能是块列表：吸收紧随其后的 `- item` 行（允许缩进）。
            let mut items = Vec::new();
            while let Some(next) = lines.peek() {
                match next.trim().strip_prefix("- ") {
                    Some(v) => {
                        items.push(unquote(v));
                        lines.next();
                    }
                    None => break,
                }
            }
            if items.is_empty() {
                FmValue::Scalar(String::new())
            } else {
                FmValue::List(items)
            }
        } else if let Some(inner) = rest.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            FmValue::List(inner.split(',').map(unquote).collect())
        } else {
            FmValue::Scalar(unquote(rest))
        };
        out.push((key, value));
    }
    out
}

/// 取 frontmatter 中指定标量字段的值（空值视为未设置）。非 frontmatter 文档返回 None。
pub fn frontmatter_field(md: &str, key: &str) -> Option<String> {
    parse_frontmatter(md)
        .into_iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| match v {
            FmValue::Scalar(s) if !s.is_empty() => Some(s),
            _ => None,
        })
}

fn first_meaningful_line(md: &str) -> String {
    for line in md.lines() {
        let t = line.trim().trim_start_matches('#').trim();
        if !t.is_empty() && !t.starts_with('>') {
            return t.chars().take(160).collect();
        }
    }
    String::new()
}

/// 按字符数（非字节）截断，避免切坏多字节字符；超长时补省略号。
fn clip(s: &str, n: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= n {
        return t.to_string();
    }
    let mut out: String = t.chars().take(n).collect();
    out.push('…');
    out
}

/// 技能包清单。本地载体是说明书（SKILL.md / AGENT.md）头部的 YAML frontmatter，
/// 历史 `tsk-skill.json` 兼容读取（frontmatter 字段优先）。SKILL.md 正文单独承载。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SkillManifest {
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    /// 逻辑分类：skill（技能）/ kb（知识库）/ toolchain（工具链）/ agent（智能体）。
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default)]
    pub description: String,
    /// 自由标签（多选，如 `oa`、`工时`），用于市场搜索与展示的 `#hashtag`。
    #[serde(default)]
    pub tags: Vec<String>,
    /// 受管标签名（多选，如「官方」「社区」）。须为后台已存在的标签，发布时按名关联。
    #[serde(default)]
    pub labels: Vec<String>,
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
            labels: Vec::new(),
            mcp_dependencies: Vec::new(),
            preferred_tools: Vec::new(),
        }
    }

    /// 把说明书 frontmatter 中已识别的元数据字段覆盖到清单上——**说明书即元数据载体**。
    /// frontmatter 未出现的字段保持原值：外部导入缺字段时由调用方先备好默认值
    /// （name ← 目录名、version 0.1.0、category 按说明书文件名推断）。
    pub fn apply_frontmatter(&mut self, md: &str) {
        let scalar = |v: FmValue| match v {
            FmValue::Scalar(s) if !s.is_empty() => Some(s),
            _ => None,
        };
        for (key, value) in parse_frontmatter(md) {
            match key.as_str() {
                "name" => {
                    if let Some(s) = scalar(value) {
                        self.name = s;
                    }
                }
                "version" => {
                    if let Some(s) = scalar(value) {
                        self.version = s;
                    }
                }
                "category" => {
                    if let Some(s) = scalar(value) {
                        self.category = s;
                    }
                }
                "description" => {
                    if let Some(s) = scalar(value) {
                        self.description = s;
                    }
                }
                "tags" => self.tags = value.into_list(),
                "labels" => self.labels = value.into_list(),
                "mcp_dependencies" => self.mcp_dependencies = value.into_list(),
                "preferred_tools" => self.preferred_tools = value.into_list(),
                _ => {}
            }
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

/// 「拖入压缩包创建技能」的解析结果：服务端解包（zip / tar.zst / tar.gz / 裸 tar）、
/// 剥离单层根目录、从说明书 frontmatter 解析元数据（历史 tsk-skill.json 兼容）后回吐，
/// 供 Web 端预填表单确认。归一化后的 tar.zst 压缩体已按 sha256 落盘，用户确认时以
/// `archive_sha256` 关联即可。
#[derive(Serialize, Deserialize)]
pub struct SkillInspectResp {
    /// 从压缩包解析出的清单（frontmatter 优先，缺失字段按目录名/说明书推断，name 可能为空待用户填写）。
    pub manifest: SkillManifest,
    /// 说明书（SKILL.md / AGENT.md）全文。
    pub skill_md: String,
    /// 归一化 tar.zst 压缩体的 sha256（已落盘，确认创建时回填给 upsert）。
    pub archive_sha256: String,
    /// 归一化 tar.zst 压缩体字节数。
    pub archive_size: u64,
    /// 压缩包内的文件数（供前端展示）。
    pub file_count: usize,
}

/// 版本号感知比较（非严格 semver，够「取最新版」用）：按 `.` 分段，两段均为纯数字则按
/// 数值比较，否则退回字符串比较；段数不足按空段补齐（空段 < 任何非空段）。
/// 例：`1.10.0 > 1.9.0`、`0.2 > 0.1.9`、`1.0.0 < 1.0.0.1`。
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let pa: Vec<&str> = a.trim().split('.').collect();
    let pb: Vec<&str> = b.trim().split('.').collect();
    for i in 0..pa.len().max(pb.len()) {
        let x = pa.get(i).copied().unwrap_or("");
        let y = pb.get(i).copied().unwrap_or("");
        let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(m), Ok(n)) => m.cmp(&n),
            _ => x.cmp(y),
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

/// 技能的一个已发布版本副本（版本列表接口返回）。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SkillVersionInfo {
    pub version: String,
    /// 压缩体 sha256（空表示该版本为纯文本裸说明书包）。
    #[serde(default)]
    pub archive_sha256: String,
    #[serde(default)]
    pub archive_size: u64,
    /// 该版本最近一次发布（含覆盖）时间。
    pub created_at: String,
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
    /// 点赞 / 收藏总数与压缩体累计下载次数。
    #[serde(default)]
    pub likes: i64,
    #[serde(default)]
    pub favorites: i64,
    #[serde(default)]
    pub downloads: i64,
    /// 当前查看者是否已点赞 / 收藏（未登录恒 false）。
    #[serde(default)]
    pub liked: bool,
    #[serde(default)]
    pub favorited: bool,
    /// 已发布的全部版本号（新→旧）。仅详情接口填充，列表接口为空。
    #[serde(default)]
    pub versions: Vec<String>,
    pub updated_at: String,
}

#[derive(Serialize, Deserialize)]
pub struct SkillRenameReq {
    pub new_name: String,
}

/// 点赞 / 收藏切换请求：kind 取 "like" / "favorite"，on 表示设置或取消。
#[derive(Serialize, Deserialize)]
pub struct ReactReq {
    pub kind: String,
    pub on: bool,
}

/// 点赞 / 收藏切换后的最新状态（计数 + 查看者标记），供前端就地刷新。
#[derive(Serialize, Deserialize)]
pub struct ReactResp {
    pub likes: i64,
    pub favorites: i64,
    pub liked: bool,
    pub favorited: bool,
}

/// 资源转移请求：把技能 / MCP 的归属转给另一个用户（按用户名）。
#[derive(Serialize, Deserialize)]
pub struct TransferReq {
    pub new_owner: String,
}

#[cfg(test)]
mod tests {
    use super::{compare_versions, parse_frontmatter, FmValue, SkillManifest};
    use std::cmp::Ordering::{Equal, Greater, Less};

    #[test]
    fn version_compare_numeric_aware() {
        assert_eq!(compare_versions("1.10.0", "1.9.0"), Greater); // 数值比较，非字典序
        assert_eq!(compare_versions("0.2", "0.1.9"), Greater);
        assert_eq!(compare_versions("1.0.0", "1.0"), Greater); // 缺段按空补齐，空段 < 非空段
        assert_eq!(compare_versions("1.0.0", "1.0.0.1"), Less);
        assert_eq!(compare_versions("2.0.0", "2.0.0"), Equal);
        assert_eq!(compare_versions(" 1.2.3 ", "1.2.3"), Equal); // 容忍首尾空白
        assert_eq!(compare_versions("1.0.0-beta", "1.0.0-alpha"), Greater); // 非数字段退回字符串比较
    }

    #[test]
    fn frontmatter_scalars_inline_and_block_lists() {
        let md = "---\n\
                  name: acme\n\
                  version: \"1.10\"\n\
                  tags: [a, B ]\n\
                  mcp_dependencies:\n  - alice/gh\n  - bob/ci\n\
                  allowed-tools: Bash(git:*)\n\
                  nested:\n  deep: 1\n\
                  ---\n# body";
        let kv = parse_frontmatter(md);
        let get = |k: &str| kv.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
        assert_eq!(get("name"), Some(FmValue::Scalar("acme".into())));
        // 值一律按字符串处理：1.10 不会掉进 YAML 浮点坑变成 1.1。
        assert_eq!(get("version"), Some(FmValue::Scalar("1.10".into())));
        assert_eq!(get("tags"), Some(FmValue::List(vec!["a".into(), "B".into()])));
        assert_eq!(
            get("mcp_dependencies"),
            Some(FmValue::List(vec!["alice/gh".into(), "bob/ci".into()]))
        );
        // 第三方生态的附加字段照常解析（消费方忽略即可），嵌套结构不炸。
        assert_eq!(get("allowed-tools"), Some(FmValue::Scalar("Bash(git:*)".into())));
        assert_eq!(get("nested"), Some(FmValue::Scalar(String::new())));
        // 无 frontmatter 的文档返回空。
        assert!(parse_frontmatter("# 没有头\n正文").is_empty());
    }

    #[test]
    fn frontmatter_overrides_manifest_and_keeps_defaults() {
        let mut m = SkillManifest::minimal("dir-name");
        m.apply_frontmatter("---\nname: real-name\ntags: a, b\ndescription: 说明\n---\n# t");
        assert_eq!(m.name, "real-name");
        assert_eq!(m.version, "0.1.0"); // frontmatter 未出现的字段保持默认值
        assert_eq!(m.category, "skill");
        assert_eq!(m.tags, vec!["a".to_string(), "b".into()]); // 标量逗号切分等价于内联数组
        assert_eq!(m.description, "说明");
    }
}
