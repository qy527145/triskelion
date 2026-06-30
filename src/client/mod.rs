//! `tsk` 客户端：登录、MCP 注册（交互式）、变量管理、mcp2cli 运行。

mod api;
mod config;
mod mcp2cli;
mod secrets;
mod skill;

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use dialoguer::{Confirm, Input, Select};

use api::HubClient;
use config::Config;

use crate::shared::{McpManifest, Protocol, Runtime, ToolMeta, stitch, strip_jsonc};

#[derive(Parser)]
#[command(name = "tsk", version, about = "Triskelion 客户端 — skill/mcp 托管平台 CLI")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 用户名密码登录 Hub（账号不存在时引导注册）
    Login {
        #[arg(long)]
        hub: Option<String>,
        #[arg(long)]
        username: Option<String>,
        /// 非交互场景用；缺省则交互输入或读环境变量 TSK_PASSWORD
        #[arg(long)]
        password: Option<String>,
    },
    /// 清除本地登录态
    Logout,
    /// 查看当前登录用户
    Whoami,
    /// MCP 注册管理
    Mcp {
        #[command(subcommand)]
        cmd: McpCmd,
    },
    /// 技能市场：打包 / 发布 / 检索 / 拉取技能包（含 SKILL.md 的文件夹）
    Skill {
        #[command(subcommand)]
        cmd: SkillCmd,
    },
    /// 拉取并解压一个技能包：tsk pull <owner>/<name>
    Pull {
        /// owner/name 或 name（默认当前用户）
        package: String,
        /// 解压到的父目录（默认当前目录，最终落在 <dir>/<name>）
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// 在当前目录打包技能包（tar.zst），等价于 tsk skill build
    Build {
        /// 技能根目录（默认当前目录）
        dir: Option<PathBuf>,
    },
    /// 批量导入第三方技能生态：把一个目录下的每个子文件夹（含 SKILL.md）作为技能发布到市场
    Import {
        /// 包含多个技能子文件夹的根目录
        dir: PathBuf,
        /// 归类标签：作为 tag 写入每个导入的技能，便于市场筛选（默认「社区资源」）
        #[arg(long)]
        category: Option<String>,
        /// 可见性：public（默认，直接上架市场）或 private
        #[arg(long)]
        visibility: Option<String>,
        /// 跳过逐个确认，直接导入全部
        #[arg(long)]
        yes: bool,
    },
    /// 变量（凭据）管理
    Secret {
        #[command(subcommand)]
        cmd: SecretCmd,
    },
    /// 把 MCP 当 CLI 运行：自动拉取并注入变量
    #[command(disable_help_flag = true)]
    Run {
        /// owner/name 或 name（默认当前用户）
        package: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
enum McpCmd {
    /// 交互式注册一个 MCP（可 --file mcp.json 预填）
    Add {
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long)]
        visibility: Option<String>,
        /// 跳过交互确认（配合 --file 用于自动化）
        #[arg(long)]
        yes: bool,
    },
    /// 列出名下 MCP
    List,
    /// 搜索 Hub 公开 MCP（按名称/描述/工具关键字匹配；省略关键字则列出全部）
    Search {
        /// 搜索关键字
        query: Option<String>,
    },
    /// 连接自己的 MCP 列出工具并上报，使其工具可被 search 检索
    Index {
        /// owner/name 或 name（默认当前用户）
        package: String,
    },
    /// 删除一个 MCP
    Remove { name: String },
}

#[derive(Subcommand)]
enum SecretCmd {
    /// 设置变量：tsk secret set KEY VALUE
    Set { key: String, value: String },
    List,
    Rm { key: String },
}

#[derive(Subcommand)]
enum SkillCmd {
    /// 在目录里生成技能脚手架（SKILL.md + tsk-skill.json）
    Init {
        /// 目标目录（默认当前目录）
        dir: Option<PathBuf>,
    },
    /// 本地打包技能包为 tar.zst（不联网）
    Build {
        /// 技能根目录（默认当前目录）
        dir: Option<PathBuf>,
    },
    /// 打包并发布到技能市场
    Publish {
        /// 技能根目录（默认当前目录）
        dir: Option<PathBuf>,
        /// private（默认）或 public
        #[arg(long)]
        visibility: Option<String>,
    },
    /// 列出名下全部技能（含私有）
    List,
    /// 搜索公开技能（可按 --category / --tag 过滤）
    Search {
        query: Option<String>,
        /// 逻辑分类：skill / kb / toolchain
        #[arg(long)]
        category: Option<String>,
        /// 标签
        #[arg(long)]
        tag: Option<String>,
    },
    /// 查看技能详情与 SKILL.md：tsk skill show <owner>/<name>
    Show { package: String },
    /// 拉取并解压一个技能包
    Pull {
        package: String,
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// 删除一个技能
    Remove { name: String },
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Login { hub, username, password } => cmd_login(hub, username, password),
        Cmd::Logout => cmd_logout(),
        Cmd::Whoami => cmd_whoami(),
        Cmd::Mcp { cmd } => match cmd {
            McpCmd::Add { file, visibility, yes } => cmd_mcp_add(file, visibility, yes),
            McpCmd::List => cmd_mcp_list(),
            McpCmd::Search { query } => cmd_mcp_search(query.as_deref().unwrap_or("")),
            McpCmd::Index { package } => cmd_mcp_index(&package),
            McpCmd::Remove { name } => cmd_mcp_remove(&name),
        },
        Cmd::Skill { cmd } => match cmd {
            SkillCmd::Init { dir } => skill::init(dir),
            SkillCmd::Build { dir } => skill::cmd_build(dir),
            SkillCmd::Publish { dir, visibility } => skill::publish(dir, visibility),
            SkillCmd::List => skill::list(),
            SkillCmd::Search { query, category, tag } => skill::search(
                query.as_deref().unwrap_or(""),
                category.as_deref(),
                tag.as_deref(),
            ),
            SkillCmd::Show { package } => skill::show(&package),
            SkillCmd::Pull { package, dir } => skill::pull(&package, dir),
            SkillCmd::Remove { name } => skill::remove(&name),
        },
        Cmd::Pull { package, dir } => skill::pull(&package, dir),
        Cmd::Build { dir } => skill::cmd_build(dir),
        Cmd::Import { dir, category, visibility, yes } => {
            skill::import(dir, category, visibility, yes)
        }
        Cmd::Secret { cmd } => match cmd {
            SecretCmd::Set { key, value } => cmd_secret_set(&key, &value),
            SecretCmd::List => cmd_secret_list(),
            SecretCmd::Rm { key } => cmd_secret_rm(&key),
        },
        Cmd::Run { package, args } => cmd_run(&package, &args),
    }
}

fn client(cfg: &Config) -> Result<HubClient> {
    let (hub, token) = cfg.require_auth()?;
    Ok(HubClient::new(hub, Some(token)))
}

// ---------------------------------------------------------------------------
// login
// ---------------------------------------------------------------------------

fn cmd_login(hub: Option<String>, username: Option<String>, password: Option<String>) -> Result<()> {
    let mut cfg = Config::load();
    let hub = hub
        .or(cfg.hub_url.clone())
        .ok_or_else(|| anyhow::anyhow!("请用 --hub <url> 指定 Hub 地址"))?;
    let username = match username {
        Some(u) => u,
        None => Input::<String>::new().with_prompt("用户名").interact_text()?,
    };
    let password = match password.or_else(|| std::env::var("TSK_PASSWORD").ok()) {
        Some(p) => p,
        None => rpassword::prompt_password("密码: ")?,
    };

    let hub_client = HubClient::new(&hub, None);
    let resp = match hub_client.login(&username, &password) {
        Ok(r) => r,
        Err(e) if e.status == 404 => {
            let ok = Confirm::new()
                .with_prompt(format!("用户 {username} 不存在，现在注册？"))
                .default(true)
                .interact()
                .unwrap_or(true);
            if !ok {
                bail!("已取消");
            }
            hub_client.register(&username, &password)?
        }
        Err(e) => return Err(e.into()),
    };

    cfg.hub_url = Some(hub.clone());
    cfg.token = Some(resp.token);
    cfg.username = Some(resp.username.clone());
    cfg.save()?;
    println!("✓ 已登录 {} 为 {}", hub, resp.username);
    Ok(())
}

fn cmd_logout() -> Result<()> {
    let mut cfg = Config::load();
    cfg.token = None;
    cfg.username = None;
    cfg.save()?;
    println!("✓ 已退出登录");
    Ok(())
}

fn cmd_whoami() -> Result<()> {
    let cfg = Config::load();
    match (&cfg.hub_url, &cfg.username) {
        (Some(h), Some(u)) => println!("{u} @ {h}"),
        _ => println!("(未登录)"),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// mcp
// ---------------------------------------------------------------------------

fn cmd_mcp_add(file: Option<PathBuf>, visibility: Option<String>, yes: bool) -> Result<()> {
    let cfg = Config::load();
    let api = client(&cfg)?;

    let manifest = if let Some(path) = file {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("读取 {}", path.display()))?;
        let m: McpManifest = serde_json::from_str(&strip_jsonc(&raw))
            .with_context(|| format!("解析 {}", path.display()))?;
        if yes { m } else { interactive_manifest(Some(m))? }
    } else {
        interactive_manifest(None)?
    };

    let visibility = visibility.unwrap_or_else(|| "private".into());
    let info = api.mcp_upsert(manifest, visibility)?;
    println!(
        "✓ 已注册 {}/{} (v{}, {})",
        info.owner, info.name, info.version, info.visibility
    );
    // 尽力而为：连接 MCP 列出工具并上报，使工具可被 search 检索；失败不影响注册。
    match index_mcp_tools(&api, &info.owner, &info.name) {
        Ok(n) if n > 0 => println!("  已索引 {n} 个工具用于搜索"),
        Ok(_) => {}
        Err(e) => println!(
            "  （工具索引已跳过：{e}）\n  设置变量后可执行 tsk mcp index {} 补全",
            info.name
        ),
    }
    println!("  现在可运行: tsk run {}/{} --help", info.owner, info.name);
    Ok(())
}

/// 解析运行所需 manifest：从 Hub 取原始 manifest 与调用者线上变量，
/// 叠加本地变量（**本地优先**）后在客户端完成凭据缝合。
/// 返回 (已缝合 manifest, 全部所需变量, 仍缺失变量)。
fn resolve_run(api: &HubClient, owner: &str, name: &str) -> Result<(McpManifest, Vec<String>, Vec<String>)> {
    let resolved = api.run_resolve(owner, name)?;
    let local = secrets::LocalSecrets::load();
    // 线上变量打底，本地变量覆盖（本地优先级更高）。
    let mut vars = resolved.vars.clone();
    for (k, v) in local.values_map() {
        vars.insert(k, v);
    }
    let (stitched, missing) = stitch(&resolved.manifest, &vars);
    Ok((stitched, resolved.required, missing))
}

/// 连接指定 MCP，列出工具并上报 Hub 检索索引，返回已索引数量。
fn index_mcp_tools(api: &HubClient, owner: &str, name: &str) -> Result<usize> {
    let (manifest, _required, missing) = resolve_run(api, owner, name)?;
    if !missing.is_empty() {
        bail!("缺少变量 {}", missing.join(", "));
    }
    let mut mcp = mcp2cli::McpClient::connect(&manifest)?;
    let tools = mcp.list_tools()?;
    let metas = tool_metas(&tools);
    let n = api.set_tools(name, &metas)?;
    Ok(n)
}

fn tool_metas(tools: &[mcp2cli::Tool]) -> Vec<ToolMeta> {
    tools
        .iter()
        .map(|t| ToolMeta {
            name: t.name.clone(),
            description: t.description.clone(),
            input_schema: t.input_schema.clone(),
        })
        .collect()
}

fn cmd_mcp_index(package: &str) -> Result<()> {
    let cfg = Config::load();
    let api = client(&cfg)?;
    let me = cfg.username.clone().context("未登录")?;
    // 接受 owner/name 或裸 name（owner 默认当前用户）。
    let (owner, name) = match package.split_once('/') {
        Some((o, n)) => (o.to_string(), n.to_string()),
        None => (me.clone(), package.to_string()),
    };
    if owner != me {
        bail!("只能索引自己的 MCP；你是 {me}，无法索引 {owner}/{name}");
    }
    let n = index_mcp_tools(&api, &owner, &name)?;
    println!("✓ 已索引 {n} 个工具到 {owner}/{name}");
    Ok(())
}

fn interactive_manifest(base: Option<McpManifest>) -> Result<McpManifest> {
    let b = base.unwrap_or_else(|| McpManifest {
        resource_type: "mcp".into(),
        name: String::new(),
        description: String::new(),
        version: "0.1.0".into(),
        runtime: Runtime::Remote,
        protocol: Protocol::Streamable,
        url: None,
        command: None,
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
    });
    let name: String = Input::new().with_prompt("名称").default(b.name).interact_text()?;
    let description: String = Input::new()
        .with_prompt("描述")
        .default(b.description)
        .allow_empty(true)
        .interact_text()?;
    let version: String = Input::new().with_prompt("版本").default(b.version).interact_text()?;

    let runtimes = ["local", "remote"];
    let rsel = Select::new()
        .with_prompt("运行时")
        .items(&runtimes)
        .default(if b.runtime == Runtime::Local { 0 } else { 1 })
        .interact()?;
    let runtime = if rsel == 0 { Runtime::Local } else { Runtime::Remote };

    let (protocol, url, command) = match runtime {
        Runtime::Local => {
            let cmd: String = Input::new()
                .with_prompt("启动命令 (如 uvx acemcp)")
                .default(b.command.unwrap_or_default())
                .interact_text()?;
            (Protocol::Stdio, None, Some(cmd))
        }
        Runtime::Remote => {
            let protos = ["sse", "streamable"];
            let psel = Select::new()
                .with_prompt("协议")
                .items(&protos)
                .default(if b.protocol == Protocol::Sse { 0 } else { 1 })
                .interact()?;
            let url: String = Input::new()
                .with_prompt("URL (可含 {VAR} 占位符)")
                .default(b.url.unwrap_or_default())
                .interact_text()?;
            let proto = if psel == 0 { Protocol::Sse } else { Protocol::Streamable };
            (proto, Some(url), None)
        }
    };

    Ok(McpManifest {
        resource_type: "mcp".into(),
        name,
        description,
        version,
        runtime,
        protocol,
        url,
        command,
        env: b.env,
        headers: b.headers,
    })
}

fn cmd_mcp_list() -> Result<()> {
    let cfg = Config::load();
    let api = client(&cfg)?;
    let list = api.mcp_list()?;
    if list.is_empty() {
        println!("(无 MCP)");
        return Ok(());
    }
    for m in list {
        let vars = m.manifest.required_vars();
        let vstr = if vars.is_empty() { String::new() } else { format!("  vars: {}", vars.join(",")) };
        println!(
            "{}/{}  v{}  {}  [{}/{}]{}",
            m.owner,
            m.name,
            m.version,
            m.visibility,
            m.manifest.runtime.as_str(),
            m.manifest.protocol.as_str(),
            vstr
        );
    }
    Ok(())
}

fn cmd_mcp_search(query: &str) -> Result<()> {
    let cfg = Config::load();
    let hub = cfg
        .hub_url
        .clone()
        .ok_or_else(|| anyhow::anyhow!("尚未配置 Hub，请先 tsk login --hub <url>"))?;
    // 搜索公开市场无需登录；带上已有 token 也无妨。
    let api = HubClient::new(hub, cfg.token.clone());
    let list = api.explore(query)?;
    if list.is_empty() {
        if query.is_empty() {
            println!("(市场暂无公开 MCP)");
        } else {
            println!("(无匹配 `{query}` 的公开 MCP)");
        }
        return Ok(());
    }
    if query.is_empty() {
        println!("公开 MCP（共 {}）：", list.len());
    } else {
        println!("匹配 `{query}` 的公开 MCP（共 {}）：", list.len());
    }
    for m in list {
        let desc = m.manifest.description.lines().next().unwrap_or("");
        println!(
            "  {}/{}  v{}  [{}/{}]  {}",
            m.owner,
            m.name,
            m.version,
            m.manifest.runtime.as_str(),
            m.manifest.protocol.as_str(),
            desc
        );
        if m.tools.is_empty() {
            continue;
        }
        // 命中关键字的工具优先展示，否则列出工具名概览。
        let ql = query.to_lowercase();
        let matched: Vec<&ToolMeta> = if query.is_empty() {
            Vec::new()
        } else {
            m.tools
                .iter()
                .filter(|t| {
                    t.name.to_lowercase().contains(&ql)
                        || t.description.to_lowercase().contains(&ql)
                })
                .collect()
        };
        if matched.is_empty() {
            let names: Vec<&str> = m.tools.iter().map(|t| t.name.as_str()).collect();
            println!("      工具({}): {}", m.tools.len(), names.join(", "));
        } else {
            for t in matched {
                let td = t.description.lines().next().unwrap_or("");
                println!("      ↳ 命中工具 {}: {}", t.name, td);
            }
        }
    }
    Ok(())
}

fn cmd_mcp_remove(name: &str) -> Result<()> {
    let cfg = Config::load();
    let api = client(&cfg)?;
    api.mcp_delete(name)?;
    println!("✓ 已删除 {name}");
    Ok(())
}

// ---------------------------------------------------------------------------
// secret
// ---------------------------------------------------------------------------

fn cmd_secret_set(key: &str, value: &str) -> Result<()> {
    let cfg = Config::load();
    // 总是写本地。
    let mut local = secrets::LocalSecrets::load();
    local.set(key, value);
    local.save()?;
    // 已登录则在本地之外再写线上（尽力而为，失败不影响本地）。
    if cfg.logged_in() {
        match client(&cfg).and_then(|api| api.secret_set(key, value).map_err(Into::into)) {
            Ok(_) => println!("✓ 已设置变量 {key}（本地 + 线上）"),
            Err(e) => println!("✓ 已写入本地变量 {key}；线上写入失败：{e}"),
        }
    } else {
        println!("✓ 已设置本地变量 {key}（未登录，仅本地）");
    }
    Ok(())
}

fn cmd_secret_list() -> Result<()> {
    let cfg = Config::load();
    let local = secrets::LocalSecrets::load();
    // 线上变量（仅已登录时；获取失败降级为仅本地）。
    let online: Vec<String> = if cfg.logged_in() {
        match client(&cfg).and_then(|api| api.secret_list().map_err(Into::into)) {
            Ok(list) => list.into_iter().map(|s| s.key).collect(),
            Err(e) => {
                eprintln!("（线上变量获取失败，仅显示本地：{e}）");
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    use std::collections::BTreeSet;
    let online_set: BTreeSet<&String> = online.iter().collect();
    let mut keys: BTreeSet<String> = BTreeSet::new();
    keys.extend(local.keys().cloned());
    keys.extend(online.iter().cloned());
    if keys.is_empty() {
        println!("(无变量)");
        return Ok(());
    }
    for k in &keys {
        let in_local = local.get(k).is_some();
        let in_online = online_set.contains(k);
        let src = match (in_local, in_online) {
            (true, true) => "本地(覆盖线上)",
            (true, false) => "本地",
            _ => "线上",
        };
        println!("{k}  [{src}]");
    }
    Ok(())
}

fn cmd_secret_rm(key: &str) -> Result<()> {
    let cfg = Config::load();
    let mut local = secrets::LocalSecrets::load();
    let removed_local = local.remove(key);
    if removed_local {
        local.save()?;
    }
    let mut removed_online = false;
    if cfg.logged_in() {
        if let Ok(api) = client(&cfg) {
            match api.secret_delete(key) {
                Ok(_) => removed_online = true,
                Err(e) if e.status == 404 => {}
                Err(e) => println!("（线上删除失败：{e}）"),
            }
        }
    }
    if removed_local || removed_online {
        println!("✓ 已删除变量 {key}");
    } else {
        println!("变量 {key} 不存在");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// run（mcp2cli）
// ---------------------------------------------------------------------------

fn cmd_run(package: &str, args: &[String]) -> Result<()> {
    let cfg = Config::load();
    let hub = cfg.hub().ok_or_else(|| {
        anyhow::anyhow!("尚未配置 Hub：请 tsk login --hub <url>，或设置 TRISKELION_HUB 环境变量")
    })?;
    // 登录非必需：带上 token（若有）即可访问私有/线上变量，无 token 也能跑公开 MCP。
    let api = HubClient::new(hub, cfg.token.clone());
    let (owner, name) = match package.split_once('/') {
        Some((o, n)) => (o.to_string(), n.to_string()),
        None => {
            let me = cfg.username.clone().ok_or_else(|| {
                anyhow::anyhow!("未登录时请用 owner/name 形式指定，如 tsk run alice/foo")
            })?;
            (me, package.to_string())
        }
    };

    let (manifest, required, missing) = resolve_run(&api, &owner, &name)?;
    if !missing.is_empty() {
        let pkg = format!("{owner}/{name}");
        eprintln!("`{pkg}` 依赖以下变量：");
        for v in &required {
            let ok = !missing.contains(v);
            eprintln!("  {} {v}", if ok { "✓ 已设置" } else { "✗ 未设置" });
        }
        eprintln!("\n缺少 {} 个变量，请先设置后重试：", missing.len());
        for v in &missing {
            eprintln!("  tsk secret set {v} <value>");
        }
        bail!("{pkg} 运行所需变量未配置完整");
    }

    let mut mcp = mcp2cli::McpClient::connect(&manifest)?;
    let tools = mcp.list_tools()?;
    let pkg = format!("{owner}/{name}");

    // 自己的 MCP（且已登录）：顺带把实时工具清单回传 Hub 作检索索引（尽力而为）。
    if cfg.logged_in() && cfg.username.as_deref() == Some(owner.as_str()) {
        let _ = api.set_tools(&name, &tool_metas(&tools));
    }

    let is_help = |s: &str| matches!(s, "--help" | "-h" | "help");
    if args.is_empty() || is_help(&args[0]) {
        mcp2cli::overview(&pkg, &name, &tools);
        return Ok(());
    }

    let tool_name = &args[0];
    let tool = tools
        .iter()
        .find(|t| &t.name == tool_name)
        .ok_or_else(|| anyhow::anyhow!("未知工具 `{tool_name}`，运行 tsk run {pkg} --help 查看"))?;
    let rest = &args[1..];
    if rest.iter().any(|a| is_help(a)) {
        mcp2cli::tool_help(&pkg, tool);
        return Ok(());
    }

    let arguments = mcp2cli::build_arguments(&tool.input_schema, rest)?;
    let started = std::time::Instant::now();
    let outcome = mcp.call_tool(tool_name, arguments);
    let ms = started.elapsed().as_millis() as i64;
    // 把调用结果回传 Hub 作审计统计（尽力而为；仅已登录时上报，匿名跑公开 MCP 不记账）。
    if cfg.logged_in() {
        let (ok, err, summary) = match &outcome {
            Ok(v) => (true, String::new(), summarize_result(v)),
            Err(e) => (false, format!("{e:#}"), String::new()),
        };
        let _ = api.report_call(&owner, &name, tool_name, ok, &err, ms, &summary);
    }
    let result = outcome?;
    if !mcp2cli::print_result(&result) {
        std::process::exit(1);
    }
    Ok(())
}

/// 把一次工具调用结果压缩成单行摘要，回传 Hub 供审计面板「结果摘要」列展示。
/// 优先拼接 `content[].text`，否则回退紧凑 JSON；统一截断到 240 字符。
fn summarize_result(result: &serde_json::Value) -> String {
    let mut s = String::new();
    if let Some(items) = result.get("content").and_then(|c| c.as_array()) {
        for item in items {
            if let Some(t) = item.get("text").and_then(|t| t.as_str()) {
                if !s.is_empty() {
                    s.push(' ');
                }
                s.push_str(t);
            }
        }
    }
    if s.trim().is_empty() {
        s = result.to_string();
    }
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = s.trim();
    if trimmed.chars().count() <= 240 {
        trimmed.to_string()
    } else {
        let mut out: String = trimmed.chars().take(240).collect();
        out.push('…');
        out
    }
}
