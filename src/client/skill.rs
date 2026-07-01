//! `tsk skill`：技能包的本地打包（build）、发布（publish）、检索（search）、
//! 拉取（pull）与查看。技能包是一个文件夹，必须含 `SKILL.md`；元数据放在
//! `tsk-skill.json`。发布前由 `tsk build` 打成 tar.zst 压缩体（zstd），服务端只收元数据
//! 与 SKILL.md，庞大的数据体以压缩包形式承载。

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::archive::{self, Format, ZSTD_LEVEL};
use crate::shared::{doc_filename, SkillInfo, SkillManifest, SKILL_CATEGORIES};

use super::api::HubClient;
use super::config::Config;

/// 默认（skill/kb/toolchain）分类的能力说明书文件名。agent 分类用 `AGENT.md`，
/// 由 `crate::shared::doc_filename` 按分类决定。
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

pub fn init(dir: Option<PathBuf>, category: Option<String>) -> Result<()> {
    let dir = dir.unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&dir).with_context(|| format!("创建目录 {}", dir.display()))?;
    let name = dir
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "my-skill".into());

    let category = category
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "skill".into());
    if !SKILL_CATEGORIES.contains(&category.as_str()) {
        bail!("category `{}` 非法，只能是 {}", category, SKILL_CATEGORIES.join(" / "));
    }
    // 说明书文件名随分类而定：agent → AGENT.md，其余 → SKILL.md。
    let doc = doc_filename(&category);

    let manifest_path = dir.join(MANIFEST);
    if manifest_path.exists() {
        println!("• 已存在 {MANIFEST}，跳过");
    } else {
        let m = SkillManifest {
            name: name.clone(),
            version: "0.1.0".into(),
            category: category.clone(),
            description: "一句话描述这个技能能干什么".into(),
            tags: vec![],
            labels: vec![],
            mcp_dependencies: vec![],
            preferred_tools: vec![],
        };
        std::fs::write(&manifest_path, serde_json::to_string_pretty(&m)? + "\n")
            .with_context(|| format!("写入 {}", manifest_path.display()))?;
        println!("✓ 生成 {}", manifest_path.display());
    }

    let skill_path = dir.join(doc);
    if skill_path.exists() {
        println!("• 已存在 {doc}，跳过");
    } else {
        std::fs::write(&skill_path, skill_md_template(&name, doc))
            .with_context(|| format!("写入 {}", skill_path.display()))?;
        println!("✓ 生成 {}", skill_path.display());
    }

    println!("\n下一步：");
    println!("  1) 编辑 {doc} 与 {MANIFEST}");
    println!("  2) tsk build           # 本地打包校验");
    println!("  3) tsk skill publish   # 发布到技能市场");
    Ok(())
}

fn skill_md_template(name: &str, doc: &str) -> String {
    format!(
        "# {name}\n\n\
> 一份「能力说明书」。Agent 初始化时只读这几百 Token 的 {doc}，按需才触发 CLI。\n\n\
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
        doc = doc,
        MANIFEST = MANIFEST,
    )
}

// ---------------------------------------------------------------------------
// build：打包成 tar.zst
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

/// 导入时对每个技能清单的批量覆盖：统一改分类、追加标签/受管标签。
#[derive(Default)]
pub struct Overrides {
    /// 覆盖逻辑分类（skill / kb / toolchain / agent）；None 则保留各自清单值。
    pub category: Option<String>,
    /// 追加的自由标签（去重合并进 manifest.tags）。
    pub tags: Vec<String>,
    /// 追加的受管标签名（去重合并进 manifest.labels，服务端按名关联）。
    pub labels: Vec<String>,
}

/// 读取目录、校验、打包。不联网。
pub fn build(dir: &Path) -> Result<Build> {
    build_with(dir, &Overrides::default())
}

/// 同 [`build`]，但在读取清单后按 `ov` 覆盖分类 / 追加标签，再据（可能被覆盖的）分类选说明书。
pub fn build_with(dir: &Path, ov: &Overrides) -> Result<Build> {
    let dir = if dir.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        dir.to_path_buf()
    };

    // 先确定分类，再据此决定必须包含的说明书文件（agent → AGENT.md，其余 → SKILL.md）。
    let mut manifest = load_manifest(&dir)?;
    // 应用导入覆盖：分类覆盖须在选说明书之前，标签/受管标签去重合并。
    if let Some(cat) = &ov.category {
        manifest.category = cat.clone();
    }
    for t in &ov.tags {
        if !t.trim().is_empty() && !manifest.tags.iter().any(|x| x.eq_ignore_ascii_case(t)) {
            manifest.tags.push(t.clone());
        }
    }
    for l in &ov.labels {
        if !l.trim().is_empty() && !manifest.labels.iter().any(|x| x == l) {
            manifest.labels.push(l.clone());
        }
    }
    validate_manifest(&manifest)?;
    let doc = doc_filename(&manifest.category);

    let skill_md_path = dir.join(doc);
    if !skill_md_path.exists() {
        bail!(
            "{} 分类的技能根目录缺少 {doc}（{}）。先运行 tsk skill init",
            manifest.category,
            dir.display()
        );
    }
    let skill_md = std::fs::read_to_string(&skill_md_path)
        .with_context(|| format!("读取 {}", skill_md_path.display()))?;

    if manifest.description.trim().is_empty() {
        manifest.description = extract_description(&skill_md);
    }

    // 收集要打包的文件（相对路径）。
    let mut files = Vec::new();
    collect_files(&dir, &dir, &mut files)?;
    files.sort();
    if !files.iter().any(|p| p == Path::new(doc)) {
        bail!("打包后未包含 {doc}（被忽略规则排除了？）");
    }

    // tar -> zstd（高压缩比、解压快；大包用多线程编码摊薄耗时）。
    let mut encoder = zstd::stream::Encoder::new(Vec::new(), ZSTD_LEVEL)
        .context("初始化 zstd 编码器")?;
    // 多线程编码：失败（如未编入 MT 支持）则回退单线程，不影响产物正确性。
    let workers = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);
    let _ = encoder.multithread(workers);
    let mut tar = tar::Builder::new(encoder);
    for rel in &files {
        let full = dir.join(rel);
        tar.append_path_with_name(&full, rel)
            .with_context(|| format!("打包 {}", full.display()))?;
    }
    let encoder = tar.into_inner().context("收尾 tar")?;
    let archive = encoder.finish().context("收尾 zstd")?;

    let sha256 = sha256_hex(&archive);
    let size = archive.len() as u64;

    let build_dir = dir.join(BUILD_DIR);
    std::fs::create_dir_all(&build_dir)
        .with_context(|| format!("创建 {}", build_dir.display()))?;
    let out_path = build_dir.join(format!("{}-{}.tar.zst", manifest.name, manifest.version));
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
// import：批量导入第三方技能生态（一个目录下的多个技能子文件夹）
// ---------------------------------------------------------------------------

/// 把 `root` 下每个含 `SKILL.md` / `AGENT.md` 的子文件夹作为技能发布到市场。
/// 三个分类维度可统一覆盖：`category`（逻辑分类，单选，覆盖各清单）、`tags`（自由标签，
/// 多选，追加）、`labels`（受管标签名如「官方」「社区」，多选，追加）。`visibility` 默认 public。
pub fn import(
    root: PathBuf,
    category: Option<String>,
    tags: Vec<String>,
    labels: Vec<String>,
    visibility: Option<String>,
    yes: bool,
) -> Result<()> {
    let cfg = Config::load();
    let api = client(&cfg)?;

    if !root.is_dir() {
        bail!("{} 不是目录", root.display());
    }
    // `--category` 是「逻辑分类」（单选），只能取 SKILL_CATEGORIES；早失败给出明确指引，
    // 避免与旧版行为（曾把 category 当自由标签写入）混淆。
    let category = category.map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
    if let Some(cat) = &category {
        if !SKILL_CATEGORIES.contains(&cat.as_str()) {
            bail!(
                "--category 只能是 {}（逻辑分类，单选）。\n\
                 若想按「社区/官方」归类请用 --label，按自由关键词请用 --tag。",
                SKILL_CATEGORIES.join(" / ")
            );
        }
    }
    let tags: Vec<String> = tags.into_iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
    let labels: Vec<String> = labels.into_iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    let visibility = visibility.unwrap_or_else(|| "public".into());

    // 收集直接子目录里含说明书（SKILL.md 或 AGENT.md）的技能文件夹（按名称排序，结果稳定）。
    let mut candidates: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(&root).with_context(|| format!("读取目录 {}", root.display()))? {
        let path = entry?.path();
        if path.is_dir() && has_doc(&path) {
            candidates.push(path);
        }
    }
    candidates.sort();

    if candidates.is_empty() {
        // 兜底：目录本身就是一个技能（直接含说明书）。
        if has_doc(&root) {
            candidates.push(root.clone());
        } else {
            bail!(
                "{} 下未发现任何含 SKILL.md / AGENT.md 的技能子文件夹",
                root.display()
            );
        }
    }

    println!("发现 {} 个技能，将以 [{}] 可见性导入：", candidates.len(), visibility);
    println!("  分类(category): {}", category.as_deref().unwrap_or("<保留各清单>"));
    println!("  标签(tags): {}", if tags.is_empty() { "<无>".into() } else { tags.join(", ") });
    println!("  受管标签(labels): {}", if labels.is_empty() { "<无>".into() } else { labels.join(", ") });
    for p in &candidates {
        println!("  • {}", p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    }
    if !yes
        && !dialoguer::Confirm::new()
            .with_prompt("确认导入以上全部技能？")
            .default(true)
            .interact()
            .unwrap_or(false)
    {
        bail!("已取消");
    }

    let ov = Overrides {
        category: category.clone(),
        tags: tags.clone(),
        labels: labels.clone(),
    };
    let mut ok = 0usize;
    let mut failed: Vec<String> = Vec::new();
    for dir in &candidates {
        let label = dir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| dir.display().to_string());
        match import_one(&api, dir, &ov, &visibility) {
            Ok(info) => {
                ok += 1;
                println!("  ✓ {}/{} v{} [{}/{}]", info.owner, info.name, info.version, info.category, info.visibility);
            }
            Err(e) => {
                failed.push(format!("{label}: {e:#}"));
                println!("  ✗ {label}：{e:#}");
            }
        }
    }

    println!("\n导入完成：成功 {ok} / 共 {}", candidates.len());
    if !failed.is_empty() {
        println!("失败 {} 个：", failed.len());
        for f in &failed {
            println!("  - {f}");
        }
    }
    Ok(())
}

/// 导入单个技能文件夹：按覆盖项打包（改分类 / 追加标签）→ 上传元数据 + 压缩体。
fn import_one(
    api: &HubClient,
    dir: &Path,
    ov: &Overrides,
    visibility: &str,
) -> Result<SkillInfo> {
    let b = build_with(dir, ov)?;
    let info = api.skill_upsert(&b.manifest, visibility, &b.skill_md, &b.sha256, b.size)?;
    if b.size > 0 {
        api.skill_archive_put(&info.owner, &info.name, b.archive)?;
    }
    Ok(info)
}



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
    let doc = doc_filename(&s.category);
    println!("\n----- {doc} -----\n{}", s.skill_md);
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
        unpack_archive(&bytes, &target)
            .with_context(|| format!("解压到 {}", target.display()))?;
    } else {
        // 纯文本裸说明书包：服务端只有说明书文本，按分类写回 SKILL.md / AGENT.md。
        let doc = doc_filename(&s.category);
        std::fs::write(target.join(doc), &s.skill_md)
            .with_context(|| format!("写入 {}", target.join(doc).display()))?;
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

/// 解压 tar 压缩体到目标目录，按魔数自动识别 zstd（新）或 gzip（历史遗留）。
fn unpack_archive(bytes: &[u8], target: &Path) -> Result<()> {
    match archive::detect(bytes) {
        Format::Zstd => {
            let dec = zstd::stream::Decoder::new(bytes).context("初始化 zstd 解码器")?;
            tar::Archive::new(dec).unpack(target)?;
        }
        Format::Gzip => {
            let dec = flate2::read::GzDecoder::new(bytes);
            tar::Archive::new(dec).unpack(target)?;
        }
        Format::Unknown => {
            // 兜底：可能是未压缩的裸 tar，尝试直接解包。
            tar::Archive::new(bytes).unpack(target)?;
        }
    }
    Ok(())
}

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
        let mut m = SkillManifest::minimal(name.clone());
        // 无清单时按实际存在的说明书推断分类：仅有 AGENT.md（无 SKILL.md）则归为 agent。
        if !dir.join(SKILL_MD).is_file() && dir.join(doc_filename("agent")).is_file() {
            m.category = "agent".into();
        }
        println!(
            "• 未找到 {MANIFEST}，使用目录名 `{name}` 作为技能名（分类 {}）",
            m.category
        );
        Ok(m)
    }
}

/// 判断目录是否含可发布的说明书（SKILL.md 或 AGENT.md）。
fn has_doc(dir: &Path) -> bool {
    dir.join(SKILL_MD).is_file() || dir.join(doc_filename("agent")).is_file()
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

/// 从 SKILL.md 提取一句话描述：优先解析 YAML frontmatter 的 `description:` 字段
/// （Anthropic 风格技能包惯用 `--- name/description ---` 头），否则退回首个有意义的正文行。
fn extract_description(md: &str) -> String {
    if let Some(d) = frontmatter_field(md, "description") {
        return clip(&d, 240);
    }
    first_meaningful_line(md)
}

/// 解析以 `---` 围起的 YAML frontmatter，取指定标量字段的值（仅支持单行标量，足够覆盖
/// `name:` / `description:` 这类常见头字段）。非 frontmatter 文档返回 None。
fn frontmatter_field(md: &str, key: &str) -> Option<String> {
    let mut lines = md.lines();
    if lines.next().map(str::trim) != Some("---") {
        return None;
    }
    let prefix = format!("{key}:");
    for line in lines {
        let t = line.trim_end();
        let trimmed = t.trim();
        if trimmed == "---" || trimmed == "..." {
            break;
        }
        if let Some(rest) = t.strip_prefix(&prefix) {
            let v = rest.trim().trim_matches('"').trim_matches('\'').trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
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
