//! `tsk skill`：技能包的本地打包（build）、发布（publish）、检索（search）、
//! 拉取（pull）与查看。技能包是一个文件夹，必须含 `SKILL.md`；元数据放在
//! `tsk-skill.json`。发布前由 `tsk build` 打成 tar.gz 压缩体，服务端只收元数据
//! 与 SKILL.md，庞大的数据体以压缩包形式承载。

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};

use crate::shared::{SkillInfo, SkillManifest, SKILL_CATEGORIES};

use super::api::HubClient;
use super::config::Config;

/// 技能根目录里必须存在的能力说明书。
const SKILL_MD: &str = "SKILL.md";
/// 技能元数据清单文件。
const MANIFEST: &str = "tsk-skill.json";
/// 构建产物目录（打包压缩体落在这里）。
const BUILD_DIR: &str = ".tsk/dist";
/// 打包时跳过的目录名。
const IGNORE_DIRS: &[&str] = &[".git", ".tsk", "node_modules", "target", ".DS_Store"];

fn client(cfg: &Config) -> Result<HubClient> {
    let (hub, token) = cfg.require_auth()?;
    Ok(HubClient::new(hub, Some(token)))
}

// ---------------------------------------------------------------------------
// init：脚手架
// ---------------------------------------------------------------------------

pub fn init(dir: Option<PathBuf>) -> Result<()> {
    let dir = dir.unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&dir).with_context(|| format!("创建目录 {}", dir.display()))?;
    let name = dir
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "my-skill".into());

    let manifest_path = dir.join(MANIFEST);
    if manifest_path.exists() {
        println!("• 已存在 {MANIFEST}，跳过");
    } else {
        let m = SkillManifest {
            name: name.clone(),
            version: "0.1.0".into(),
            category: "skill".into(),
            description: "一句话描述这个技能能干什么".into(),
            tags: vec![],
            mcp_dependencies: vec![],
            preferred_tools: vec![],
        };
        std::fs::write(&manifest_path, serde_json::to_string_pretty(&m)? + "\n")
            .with_context(|| format!("写入 {}", manifest_path.display()))?;
        println!("✓ 生成 {}", manifest_path.display());
    }

    let skill_path = dir.join(SKILL_MD);
    if skill_path.exists() {
        println!("• 已存在 {SKILL_MD}，跳过");
    } else {
        std::fs::write(&skill_path, skill_md_template(&name))
            .with_context(|| format!("写入 {}", skill_path.display()))?;
        println!("✓ 生成 {}", skill_path.display());
    }

    println!("\n下一步：");
    println!("  1) 编辑 {SKILL_MD} 与 {MANIFEST}");
    println!("  2) tsk build           # 本地打包校验");
    println!("  3) tsk skill publish   # 发布到技能市场");
    Ok(())
}

fn skill_md_template(name: &str) -> String {
    format!(
        "# {name}\n\n\
> 一份「能力说明书」。Agent 初始化时只读这几百 Token 的 SKILL.md，按需才触发 CLI。\n\n\
## 能力概述\n\n\
描述这个技能解决什么问题、适用场景。\n\n\
## 使用方式\n\n\
若本技能为纯文本裸说明书，直接按下文步骤操作即可。\n\n\
## 依赖的 MCP（可选）\n\n\
若本技能依赖底层 MCP 工具，用 `tsk` 包装调用。例如依赖 `alice/github-inspector`：\n\n\
```bash\n\
# 查看可用工具\n\
tsk run alice/github-inspector --help\n\
# 调用某个工具（倾向优先使用 create_issue / list_prs）\n\
tsk run alice/github-inspector create_issue --title \"...\" --body \"...\"\n\
```\n\n\
在 {MANIFEST} 的 `mcp_dependencies` 里登记依赖，在 `preferred_tools` 里写明倾向使用的工具。\n",
        name = name,
        MANIFEST = MANIFEST,
    )
}

// ---------------------------------------------------------------------------
// build：打包成 tar.gz
// ---------------------------------------------------------------------------

/// 一次构建的产物。
pub struct Build {
    pub manifest: SkillManifest,
    pub skill_md: String,
    pub archive: Vec<u8>,
    pub sha256: String,
    pub size: u64,
    pub file_count: usize,
    pub out_path: PathBuf,
}

/// 读取目录、校验、打包。不联网。
pub fn build(dir: &Path) -> Result<Build> {
    let dir = if dir.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        dir.to_path_buf()
    };
    let skill_md_path = dir.join(SKILL_MD);
    if !skill_md_path.exists() {
        bail!(
            "技能根目录缺少 {SKILL_MD}（{}）。先运行 tsk skill init",
            dir.display()
        );
    }
    let skill_md = std::fs::read_to_string(&skill_md_path)
        .with_context(|| format!("读取 {}", skill_md_path.display()))?;

    let mut manifest = load_manifest(&dir)?;
    if manifest.description.trim().is_empty() {
        manifest.description = first_meaningful_line(&skill_md);
    }
    validate_manifest(&manifest)?;

    // 收集要打包的文件（相对路径）。
    let mut files = Vec::new();
    collect_files(&dir, &dir, &mut files)?;
    files.sort();
    if !files.iter().any(|p| p == Path::new(SKILL_MD)) {
        bail!("打包后未包含 {SKILL_MD}（被忽略规则排除了？）");
    }

    // tar -> gzip。
    let mut tar = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
    for rel in &files {
        let full = dir.join(rel);
        tar.append_path_with_name(&full, rel)
            .with_context(|| format!("打包 {}", full.display()))?;
    }
    let gz = tar.into_inner().context("收尾 tar")?;
    let archive = gz.finish().context("收尾 gzip")?;

    let sha256 = sha256_hex(&archive);
    let size = archive.len() as u64;

    let build_dir = dir.join(BUILD_DIR);
    std::fs::create_dir_all(&build_dir)
        .with_context(|| format!("创建 {}", build_dir.display()))?;
    let out_path = build_dir.join(format!("{}-{}.tar.gz", manifest.name, manifest.version));
    std::fs::write(&out_path, &archive)
        .with_context(|| format!("写入 {}", out_path.display()))?;

    Ok(Build {
        manifest,
        skill_md,
        archive,
        sha256,
        size,
        file_count: files.len(),
        out_path,
    })
}

pub fn cmd_build(dir: Option<PathBuf>) -> Result<()> {
    let b = build(&dir.unwrap_or_else(|| PathBuf::from(".")))?;
    print_build(&b);
    Ok(())
}

fn print_build(b: &Build) {
    println!("✓ 已构建技能 {} v{} [{}]", b.manifest.name, b.manifest.version, b.manifest.category);
    println!("  文件数: {}", b.file_count);
    println!("  压缩体: {} ({})", b.out_path.display(), human_size(b.size));
    println!("  sha256: {}", b.sha256);
    if !b.manifest.mcp_dependencies.is_empty() {
        println!("  依赖 MCP: {}", b.manifest.mcp_dependencies.join(", "));
    }
    if !b.manifest.tags.is_empty() {
        println!("  标签: {}", b.manifest.tags.join(", "));
    }
}

// ---------------------------------------------------------------------------
// publish：build + 上传元数据 + 上传压缩体
// ---------------------------------------------------------------------------

pub fn publish(dir: Option<PathBuf>, visibility: Option<String>) -> Result<()> {
    let cfg = Config::load();
    let api = client(&cfg)?;
    let b = build(&dir.unwrap_or_else(|| PathBuf::from(".")))?;
    print_build(&b);

    let visibility = visibility.unwrap_or_else(|| "private".into());
    println!("\n上传元数据与 SKILL.md…");
    let info = api.skill_upsert(&b.manifest, &visibility, &b.skill_md, &b.sha256, b.size)?;
    println!("上传压缩体 ({})…", human_size(b.size));
    api.skill_archive_put(&info.owner, &info.name, b.archive.clone())?;

    println!(
        "✓ 已发布 {}/{} v{} [{}/{}]",
        info.owner, info.name, info.version, info.category, info.visibility
    );
    println!("  拉取: tsk pull {}/{}", info.owner, info.name);
    if info.visibility != "public" {
        println!("  （当前为 private，仅自己可见；在 Web 端或重新 publish --visibility public 即可上架）");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// list / search / show / remove
// ---------------------------------------------------------------------------

pub fn list() -> Result<()> {
    let cfg = Config::load();
    let api = client(&cfg)?;
    let items = api.skill_list()?;
    if items.is_empty() {
        println!("(无技能。tsk skill init 新建一个)");
        return Ok(());
    }
    for s in items {
        print_skill_line(&s);
    }
    Ok(())
}

pub fn search(query: &str, category: Option<&str>, tag: Option<&str>) -> Result<()> {
    let cfg = Config::load();
    let hub = cfg
        .hub_url
        .clone()
        .ok_or_else(|| anyhow::anyhow!("尚未配置 Hub，请先 tsk login --hub <url>"))?;
    let api = HubClient::new(hub, cfg.token.clone());
    let items = api.skill_explore(query, category, tag)?;
    if items.is_empty() {
        println!("(无匹配的公开技能)");
        return Ok(());
    }
    println!("公开技能（共 {}）：", items.len());
    for s in items {
        print_skill_line(&s);
        let desc = s.description.lines().next().unwrap_or("");
        if !desc.is_empty() {
            println!("      {desc}");
        }
        if !s.mcp_dependencies.is_empty() {
            println!("      依赖 MCP: {}", s.mcp_dependencies.join(", "));
        }
    }
    Ok(())
}

pub fn show(package: &str) -> Result<()> {
    let cfg = Config::load();
    let (owner, name) = split_package(package, &cfg)?;
    let api = HubClient::new(
        cfg.hub_url
            .clone()
            .ok_or_else(|| anyhow::anyhow!("尚未配置 Hub，请先 tsk login --hub <url>"))?,
        cfg.token.clone(),
    );
    let s = api.skill_get(&owner, &name)?;
    println!("# {}/{}  v{}  [{}/{}]", s.owner, s.name, s.version, s.category, s.visibility);
    if !s.tags.is_empty() {
        println!("标签: {}", s.tags.join(", "));
    }
    if !s.mcp_dependencies.is_empty() {
        println!("依赖 MCP: {}", s.mcp_dependencies.join(", "));
    }
    if !s.preferred_tools.is_empty() {
        println!("倾向工具: {}", s.preferred_tools.join(", "));
    }
    if s.archive_size > 0 {
        println!("压缩体: {} (sha256 {})", human_size(s.archive_size), &s.archive_sha256[..s.archive_sha256.len().min(12)]);
    }
    println!("\n----- SKILL.md -----\n{}", s.skill_md);
    Ok(())
}

pub fn remove(name: &str) -> Result<()> {
    let cfg = Config::load();
    let api = client(&cfg)?;
    let me = cfg.username.clone().context("未登录")?;
    api.skill_delete(&me, name)?;
    println!("✓ 已删除技能 {name}");
    Ok(())
}

// ---------------------------------------------------------------------------
// pull：下载压缩体并解压
// ---------------------------------------------------------------------------

pub fn pull(package: &str, dir: Option<PathBuf>) -> Result<()> {
    let cfg = Config::load();
    let (owner, name) = split_package(package, &cfg)?;
    let api = HubClient::new(
        cfg.hub_url
            .clone()
            .ok_or_else(|| anyhow::anyhow!("尚未配置 Hub，请先 tsk login --hub <url>"))?,
        cfg.token.clone(),
    );
    let s = api.skill_get(&owner, &name)?;

    let base = dir.unwrap_or_else(|| PathBuf::from("."));
    let target = base.join(&name);
    if target.exists() {
        bail!("目标目录已存在：{}（换 --dir 或先删除）", target.display());
    }
    std::fs::create_dir_all(&target)
        .with_context(|| format!("创建 {}", target.display()))?;

    if s.archive_size > 0 && !s.archive_sha256.is_empty() {
        println!("下载压缩体 {}…", human_size(s.archive_size));
        let bytes = api.skill_archive_get(&owner, &name)?;
        let got = sha256_hex(&bytes);
        if got != s.archive_sha256 {
            bail!("压缩体校验失败：期望 {} 实得 {got}", s.archive_sha256);
        }
        let dec = GzDecoder::new(&bytes[..]);
        let mut ar = tar::Archive::new(dec);
        ar.unpack(&target)
            .with_context(|| format!("解压到 {}", target.display()))?;
    } else {
        // 纯文本裸说明书包：服务端只有 SKILL.md。
        std::fs::write(target.join(SKILL_MD), &s.skill_md)
            .with_context(|| format!("写入 {}", target.join(SKILL_MD).display()))?;
    }

    println!("✓ 已拉取 {}/{} → {}", owner, name, target.display());
    if !s.mcp_dependencies.is_empty() {
        println!("\n该技能依赖以下 MCP，用 tsk 包装调用：");
        for d in &s.mcp_dependencies {
            println!("  tsk run {d} --help");
        }
    }
    if !s.preferred_tools.is_empty() {
        println!("倾向优先使用：{}", s.preferred_tools.join(", "));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn load_manifest(dir: &Path) -> Result<SkillManifest> {
    let path = dir.join(MANIFEST);
    if path.exists() {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("读取 {}", path.display()))?;
        let m: SkillManifest = serde_json::from_str(&crate::shared::strip_jsonc(&raw))
            .with_context(|| format!("解析 {}", path.display()))?;
        Ok(m)
    } else {
        // 无清单：用目录名兜底。
        let name = dir
            .canonicalize()
            .ok()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "skill".into());
        println!("• 未找到 {MANIFEST}，使用目录名 `{name}` 作为技能名");
        Ok(SkillManifest::minimal(name))
    }
}

fn validate_manifest(m: &SkillManifest) -> Result<()> {
    if m.name.is_empty()
        || m.name.len() > 128
        || !m
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        bail!("技能名非法 `{}`：仅允许字母/数字/_-.，长度 1..=128", m.name);
    }
    if !SKILL_CATEGORIES.contains(&m.category.as_str()) {
        bail!("category `{}` 非法，只能是 {}", m.category, SKILL_CATEGORIES.join(" / "));
    }
    Ok(())
}

/// 递归收集文件，相对 root 输出路径；跳过忽略目录。
fn collect_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("读取目录 {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            if IGNORE_DIRS.contains(&name.as_ref()) {
                continue;
            }
            collect_files(root, &path, out)?;
        } else if ft.is_file() {
            if IGNORE_DIRS.contains(&name.as_ref()) {
                continue;
            }
            if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_path_buf());
            }
        }
    }
    Ok(())
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

fn split_package(package: &str, cfg: &Config) -> Result<(String, String)> {
    match package.split_once('/') {
        Some((o, n)) => Ok((o.to_string(), n.to_string())),
        None => {
            let me = cfg
                .username
                .clone()
                .context("未登录，无法推断 owner，请用 owner/name 形式")?;
            Ok((me, package.to_string()))
        }
    }
}

fn print_skill_line(s: &SkillInfo) {
    let tags = if s.tags.is_empty() {
        String::new()
    } else {
        format!("  #{}", s.tags.join(" #"))
    };
    let arch = if s.archive_size > 0 {
        format!("  📦{}", human_size(s.archive_size))
    } else {
        "  (纯文本)".into()
    };
    println!(
        "{}/{}  v{}  [{}/{}]{}{}",
        s.owner, s.name, s.version, s.category, s.visibility, arch, tags
    );
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn human_size(n: u64) -> String {
    const U: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut f = n as f64;
    let mut i = 0;
    while f >= 1024.0 && i < U.len() - 1 {
        f /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{f:.1} {}", U[i])
    }
}
