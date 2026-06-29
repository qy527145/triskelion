//! `tsk` 客户端：登录、MCP 注册（交互式）、变量管理、mcp2cli 运行。

mod api;
mod config;
mod mcp2cli;

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use dialoguer::{Confirm, Input, Select};

use api::HubClient;
use config::Config;

use crate::shared::{McpManifest, Protocol, Runtime, ToolMeta, strip_jsonc};

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

/// 连接指定 MCP，列出工具并上报 Hub 检索索引，返回已索引数量。
fn index_mcp_tools(api: &HubClient, owner: &str, name: &str) -> Result<usize> {
    let resolved = api.run_resolve(owner, name)?;
    if !resolved.missing.is_empty() {
        bail!("缺少变量 {}", resolved.missing.join(", "));
    }
    let mut mcp = mcp2cli::McpClient::connect(&resolved.manifest)?;
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
    let api = client(&cfg)?;
    api.secret_set(key, value)?;
    println!("✓ 已设置变量 {key}");
    Ok(())
}

fn cmd_secret_list() -> Result<()> {
    let cfg = Config::load();
    let api = client(&cfg)?;
    let list = api.secret_list()?;
    if list.is_empty() {
        println!("(无变量)");
        return Ok(());
    }
    for s in list {
        println!("{}  ({})", s.key, s.updated_at);
    }
    Ok(())
}

fn cmd_secret_rm(key: &str) -> Result<()> {
    let cfg = Config::load();
    let api = client(&cfg)?;
    api.secret_delete(key)?;
    println!("✓ 已删除变量 {key}");
    Ok(())
}

// ---------------------------------------------------------------------------
// run（mcp2cli）
// ---------------------------------------------------------------------------

fn cmd_run(package: &str, args: &[String]) -> Result<()> {
    let cfg = Config::load();
    let api = client(&cfg)?;
    let (owner, name) = match package.split_once('/') {
        Some((o, n)) => (o.to_string(), n.to_string()),
        None => {
            let me = cfg.username.clone().context("未登录，无法推断 owner")?;
            (me, package.to_string())
        }
    };

    let resolved = api.run_resolve(&owner, &name)?;
    if !resolved.missing.is_empty() {
        let pkg = format!("{owner}/{name}");
        eprintln!("`{pkg}` 依赖以下变量：");
        for v in &resolved.required {
            let ok = !resolved.missing.contains(v);
            eprintln!("  {} {v}", if ok { "✓ 已设置" } else { "✗ 未设置" });
        }
        eprintln!("\n缺少 {} 个变量，请先设置后重试：", resolved.missing.len());
        for v in &resolved.missing {
            eprintln!("  tsk secret set {v} <value>");
        }
        bail!("{pkg} 运行所需变量未配置完整");
    }

    let mut mcp = mcp2cli::McpClient::connect(&resolved.manifest)?;
    let tools = mcp.list_tools()?;
    let pkg = format!("{owner}/{name}");

    // 自己的 MCP：顺带把实时工具清单回传 Hub 作检索索引（尽力而为，失败不影响运行）。
    if cfg.username.as_deref() == Some(owner.as_str()) {
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
    let result = mcp.call_tool(tool_name, arguments)?;
    if !mcp2cli::print_result(&result) {
        std::process::exit(1);
    }
    Ok(())
}
