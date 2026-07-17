//! 从上传的压缩包解析并归一化为技能包（Web 端「拖入压缩包创建技能」用）。
//!
//! 支持四种入包格式：zip（用户用系统工具直接压的文件夹）、tar.zst（`tsk build` 产物）、
//! tar.gz（历史遗留）、裸 tar。服务端在内存中解包，剥离用户常见的「单层根目录」前缀
//! （把整个文件夹压进包时会多一层），从说明书（SKILL.md / AGENT.md）的 **frontmatter
//! 读取元数据**（历史 `tsk-skill.json` 兼容作缺省基底，缺失字段用默认值），
//! 再统一重打成平台原生的 **tar.zst**——否则 zip 落盘后 `tsk pull` 无法解压。

use std::io::Read;

use crate::archive::{self, Format, ZSTD_LEVEL};
use crate::shared::{doc_filename, extract_description, SkillManifest, SKILL_CATEGORIES};

use super::error::ApiError;

/// 历史技能元数据清单文件名（向后兼容读取；元数据主载体已是说明书 frontmatter）。
const MANIFEST_FILE: &str = "tsk-skill.json";
/// 解包时跳过的目录/文件名（与 client 打包侧一致，避免把无关体积打进去）。
/// `__MACOSX` 与 `.DS_Store` / AppleDouble（`._*`）是 macOS 压缩时注入的元数据噪声。
const IGNORE_NAMES: &[&str] = &[
    ".git",
    ".tsk",
    "node_modules",
    "target",
    ".DS_Store",
    "__MACOSX",
];
/// 解压后累计字节上限：防 zip/tar 炸弹撑爆内存。
const MAX_TOTAL_UNCOMPRESSED: u64 = 1024 * 1024 * 1024; // 1 GiB

/// 解包并归一化后的技能包产物。
pub struct Extracted {
    pub manifest: SkillManifest,
    pub skill_md: String,
    /// 归一化后的 tar.zst 压缩体（内容寻址落盘用）。
    pub archive: Vec<u8>,
    pub file_count: usize,
}

/// 主入口：把任意受支持格式的压缩包归一化为一个技能包。
pub fn extract_skill(bytes: &[u8]) -> Result<Extracted, ApiError> {
    // 1) 解出全部文件（含路径安全过滤：拒绝 `..` / 绝对路径 / 符号链接）。
    let files = read_entries(bytes)?;
    // 2) 先剔除忽略项与空路径（含 macOS 的 `__MACOSX` / `._*` 噪声）——务必在剥离根目录之前，
    //    否则这些散落在根部的噪声会让「唯一公共顶层目录」判定失败。
    let files: Vec<(String, Vec<u8>)> = files
        .into_iter()
        .filter(|(p, _)| !p.is_empty() && !is_ignored(p))
        .collect();
    // 3) 剥离单层（或多层）公共根目录，直到说明书回到根目录。
    let (files, wrapper) = normalize_root(files);
    if files.is_empty() {
        return Err(ApiError::bad_request("压缩包为空，或只含被忽略的文件"));
    }

    // 4) 元数据基底：历史 tsk-skill.json（兼容读取），否则按目录名/说明书推断最小清单；
    //    真正的主载体是说明书 frontmatter（下一步覆盖），缺失字段沿用基底默认值。
    let manifest_raw = files.iter().find(|(p, _)| p == MANIFEST_FILE).map(|(_, d)| d);
    let mut manifest = match manifest_raw {
        Some(data) => {
            let s = String::from_utf8_lossy(data);
            serde_json::from_str::<SkillManifest>(&crate::shared::strip_jsonc(&s))
                .map_err(|e| ApiError::bad_request(format!("解析 {MANIFEST_FILE} 失败: {e}")))?
        }
        None => infer_manifest(&files, wrapper.as_deref()),
    };
    // 分类决定说明书文件名，而分类又可能写在 frontmatter 里：先用「SKILL.md 否则
    // AGENT.md」的 category 字段定分类（单说明书场景与下一步读的是同一文件，自洽）。
    let probe = ["SKILL.md", "AGENT.md"]
        .iter()
        .find_map(|n| files.iter().find(|(p, _)| p == n));
    if let Some((_, data)) = probe {
        if let Some(cat) = crate::shared::frontmatter_field(&String::from_utf8_lossy(data), "category") {
            manifest.category = cat;
        }
    }

    if !SKILL_CATEGORIES.contains(&manifest.category.as_str()) {
        return Err(ApiError::bad_request(format!(
            "category 只能是 {}",
            SKILL_CATEGORIES.join(" / ")
        )));
    }

    // 5) 定位说明书（必需）。分类决定文件名：agent → AGENT.md，其余 → SKILL.md。
    let doc = doc_filename(&manifest.category);
    let skill_md = files
        .iter()
        .find(|(p, _)| p == doc)
        .map(|(_, d)| String::from_utf8_lossy(d).into_owned())
        .ok_or_else(|| {
            ApiError::bad_request(format!("压缩包根目录缺少 {doc}（{} 分类要求）", manifest.category))
        })?;

    // 说明书 frontmatter 覆盖元数据（name/version/tags/依赖等，未出现的字段保持基底值）。
    manifest.apply_frontmatter(&skill_md);
    if !SKILL_CATEGORIES.contains(&manifest.category.as_str()) {
        return Err(ApiError::bad_request(format!(
            "category 只能是 {}",
            SKILL_CATEGORIES.join(" / ")
        )));
    }
    // 清单/frontmatter 显式给了非法技能名才报错；为空则留给前端表单让用户填。
    if !manifest.name.is_empty() && !valid_name(&manifest.name) {
        return Err(ApiError::bad_request(
            "元数据中的技能名非法：仅允许字母、数字、_、-、.，长度 1..=128",
        ));
    }

    if manifest.description.trim().is_empty() {
        manifest.description = extract_description(&skill_md);
    }

    // 6) 重打成平台原生 tar.zst（落盘后可被 tsk pull 正常解压）。
    let archive = repack_tar_zst(&files)?;
    Ok(Extracted {
        manifest,
        skill_md,
        archive,
        file_count: files.len(),
    })
}

// ---------------------------------------------------------------------------
// 解包
// ---------------------------------------------------------------------------

/// 解出压缩包全部文件（路径→内容）。按魔数自动识别 zip / zstd / gzip / 裸 tar。
fn read_entries(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, ApiError> {
    match archive::detect(bytes) {
        Format::Zip => read_zip(bytes),
        Format::Zstd => {
            let dec = zstd::stream::Decoder::new(bytes)
                .map_err(|e| ApiError::bad_request(format!("zstd 解码失败: {e}")))?;
            read_tar(dec)
        }
        Format::Gzip => read_tar(flate2::read::GzDecoder::new(bytes)),
        Format::Unknown => {
            // 未压缩的裸 tar 在偏移 257 处有 "ustar" 魔数；否则判定为无法识别的格式。
            let is_tar = bytes.len() > 262 && &bytes[257..262] == b"ustar";
            if !is_tar {
                return Err(ApiError::bad_request(
                    "无法识别的压缩格式：请上传 zip / tar.zst / tar.gz（或未压缩 tar）",
                ));
            }
            read_tar(bytes)
        }
    }
}

/// 读取 tar 归档全部普通文件（跳过目录 / 符号链接等）。
fn read_tar<R: Read>(reader: R) -> Result<Vec<(String, Vec<u8>)>, ApiError> {
    let mut ar = tar::Archive::new(reader);
    let entries = ar
        .entries()
        .map_err(|e| ApiError::bad_request(format!("不是合法的 tar 归档: {e}")))?;
    let mut out = Vec::new();
    let mut total: u64 = 0;
    for entry in entries {
        let mut entry = entry.map_err(|e| ApiError::bad_request(format!("读取归档项失败: {e}")))?;
        if !entry.header().entry_type().is_file() {
            continue; // 目录 / 符号链接 / 硬链接一律跳过
        }
        let path = entry
            .path()
            .map_err(|e| ApiError::bad_request(format!("归档含非法路径: {e}")))?;
        let Some(rel) = safe_rel_path(&path) else {
            continue; // 跳过含 `..` / 绝对路径的项
        };
        let mut data = Vec::new();
        entry
            .read_to_end(&mut data)
            .map_err(|e| ApiError::bad_request(format!("读取归档项失败: {e}")))?;
        total = total.saturating_add(data.len() as u64);
        if total > MAX_TOTAL_UNCOMPRESSED {
            return Err(ApiError::bad_request("解压后体积过大（> 1 GiB），已拒绝"));
        }
        out.push((rel, data));
    }
    Ok(out)
}

/// 读取 zip 全部普通文件。`enclosed_name` 会拦下路径穿越（`..` / 绝对路径）。
fn read_zip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, ApiError> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| ApiError::bad_request(format!("不是合法的 zip: {e}")))?;
    let mut out = Vec::new();
    let mut total: u64 = 0;
    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|e| ApiError::bad_request(format!("读取 zip 项失败: {e}")))?;
        if file.is_dir() {
            continue;
        }
        let Some(name) = file.enclosed_name() else {
            continue; // 非法/穿越路径
        };
        let Some(rel) = safe_rel_path(&name) else {
            continue;
        };
        let mut data = Vec::new();
        file.read_to_end(&mut data)
            .map_err(|e| ApiError::bad_request(format!("读取 zip 项失败: {e}")))?;
        total = total.saturating_add(data.len() as u64);
        if total > MAX_TOTAL_UNCOMPRESSED {
            return Err(ApiError::bad_request("解压后体积过大（> 1 GiB），已拒绝"));
        }
        out.push((rel, data));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 归一化
// ---------------------------------------------------------------------------

/// 归一化路径为「安全相对路径」：只保留普通路径段，拒绝绝对路径 / `..` / Windows 盘符，
/// 统一用 `/` 分隔。返回 None 表示该项不安全，应跳过。
fn safe_rel_path(path: &std::path::Path) -> Option<String> {
    use std::path::Component;
    let mut parts = Vec::new();
    for comp in path.components() {
        match comp {
            Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => return None, // RootDir / ParentDir / Prefix 均视为不安全
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

/// 用户常把整个文件夹压进包（`my-skill/SKILL.md`），甚至多套一层（`wrapper/my-skill/...`）。
/// 反复剥离唯一的公共顶层目录，直到说明书回到根目录为止。返回 (归一化文件, 最内层被剥离的目录名)，
/// 后者可作为缺省技能名的候选。
fn normalize_root(mut files: Vec<(String, Vec<u8>)>) -> (Vec<(String, Vec<u8>)>, Option<String>) {
    let mut wrapper = None;
    for _ in 0..8 {
        // 根目录已有说明书则停止，避免误剥离真正的技能根。
        if files.iter().any(|(p, _)| p == "SKILL.md" || p == "AGENT.md") {
            break;
        }
        let Some(root) = common_root(&files) else { break };
        let prefix = format!("{root}/");
        files = files
            .into_iter()
            .filter_map(|(p, d)| p.strip_prefix(&prefix).map(|s| (s.to_string(), d)))
            .collect();
        wrapper = Some(root);
    }
    (files, wrapper)
}

/// 当且仅当所有文件都嵌在同一个顶层目录下（且该目录下确有内容）时，返回该目录名。
fn common_root(files: &[(String, Vec<u8>)]) -> Option<String> {
    let root = files.first()?.0.split('/').next()?.to_string();
    if root.is_empty() {
        return None;
    }
    let all_under = files.iter().all(|(p, _)| {
        let mut it = p.splitn(2, '/');
        it.next() == Some(root.as_str()) && it.next().is_some_and(|rest| !rest.is_empty())
    });
    if all_under {
        Some(root)
    } else {
        None
    }
}

/// 无 tsk-skill.json 时推断最小清单：技能名取被剥离的目录名（须为合法 slug），
/// 分类按存在的说明书推断（仅 AGENT.md 则 agent，否则 skill）。
fn infer_manifest(files: &[(String, Vec<u8>)], wrapper: Option<&str>) -> SkillManifest {
    let name = wrapper.filter(|w| valid_name(w)).unwrap_or("").to_string();
    let mut m = SkillManifest::minimal(name);
    let has = |n: &str| files.iter().any(|(p, _)| p == n);
    if !has("SKILL.md") && has("AGENT.md") {
        m.category = "agent".into();
    }
    m
}

/// 重打成 tar.zst（文件按路径排序，产物稳定；大包用多线程编码摊薄耗时）。
fn repack_tar_zst(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, ApiError> {
    let mut sorted: Vec<&(String, Vec<u8>)> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let mut encoder = zstd::stream::Encoder::new(Vec::new(), ZSTD_LEVEL)
        .map_err(|e| ApiError::internal(format!("初始化 zstd 编码器: {e}")))?;
    let workers = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);
    let _ = encoder.multithread(workers);
    let mut tar = tar::Builder::new(encoder);
    for (name, data) in sorted {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        tar.append_data(&mut header, name, data.as_slice())
            .map_err(|e| ApiError::internal(format!("打包 {name}: {e}")))?;
    }
    let encoder = tar
        .into_inner()
        .map_err(|e| ApiError::internal(format!("收尾 tar: {e}")))?;
    encoder
        .finish()
        .map_err(|e| ApiError::internal(format!("收尾 zstd: {e}")))
}

/// 路径任一段命中忽略清单，或为 AppleDouble 资源分叉（`._*`），即跳过。
fn is_ignored(path: &str) -> bool {
    path.split('/')
        .any(|seg| IGNORE_NAMES.contains(&seg) || seg.starts_with("._"))
}

/// 技能名校验（与 skills::valid_name 同规则）：字母/数字/_-.，长度 1..=128。
fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// 打一个 tar.zst 测试包（zstd 低级别，够快）。
    fn tar_zst(files: &[(&str, &[u8])]) -> Vec<u8> {
        let enc = zstd::stream::Encoder::new(Vec::new(), 1).unwrap();
        let mut tar = tar::Builder::new(enc);
        for &(name, data) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_mtime(0);
            tar.append_data(&mut h, name, data).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }

    /// 打一个 zip 测试包（Stored，无需压缩后端）。
    fn zip_stored(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for &(name, data) in files {
            w.start_file(name, opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    /// 解包并断言成功（ApiError 未实现 Debug，转成 message 供 unwrap 打印）。
    fn ok(bytes: &[u8]) -> Extracted {
        extract_skill(bytes).map_err(|e| e.message).unwrap()
    }

    #[test]
    fn strips_single_root_folder_and_infers_name() {
        let bytes = tar_zst(&[
            ("my-skill/SKILL.md", b"# Hi\n\nbody"),
            ("my-skill/scripts/run.sh", b"echo hi"),
        ]);
        let e = ok(&bytes);
        assert_eq!(e.manifest.name, "my-skill");
        assert_eq!(e.manifest.category, "skill");
        assert!(e.skill_md.contains("body"));
        assert_eq!(e.file_count, 2);
        // 归一化产物必须是平台原生 tar.zst。
        assert_eq!(archive::detect(&e.archive), Format::Zstd);
    }

    #[test]
    fn reads_manifest_when_present() {
        let manifest = br#"{"name":"acme","version":"1.2.3","category":"kb","tags":["a"]}"#;
        let bytes = tar_zst(&[
            ("acme/tsk-skill.json", manifest),
            ("acme/SKILL.md", b"body"),
        ]);
        let e = ok(&bytes);
        assert_eq!(e.manifest.name, "acme");
        assert_eq!(e.manifest.version, "1.2.3");
        assert_eq!(e.manifest.category, "kb");
        assert_eq!(e.manifest.tags, vec!["a".to_string()]);
    }

    #[test]
    fn frontmatter_is_manifest_carrier() {
        let md: &[u8] = b"---\nname: acme\nversion: 2.0.0\ncategory: kb\ndescription: from fm\ntags: [x, y]\nmcp_dependencies:\n  - alice/gh\n---\nbody";
        let bytes = tar_zst(&[("wrap/SKILL.md", md)]);
        let e = ok(&bytes);
        assert_eq!(e.manifest.name, "acme"); // frontmatter name 优先于剥离的目录名
        assert_eq!(e.manifest.version, "2.0.0");
        assert_eq!(e.manifest.category, "kb");
        assert_eq!(e.manifest.description, "from fm");
        assert_eq!(e.manifest.tags, vec!["x".to_string(), "y".into()]);
        assert_eq!(e.manifest.mcp_dependencies, vec!["alice/gh".to_string()]);
    }

    #[test]
    fn frontmatter_wins_over_legacy_manifest() {
        let manifest = br#"{"name":"acme","version":"1.0.0","category":"kb"}"#;
        let md: &[u8] = b"---\nversion: 3.0.0\n---\nbody";
        let bytes = tar_zst(&[("acme/tsk-skill.json", manifest), ("acme/SKILL.md", md)]);
        let e = ok(&bytes);
        assert_eq!(e.manifest.version, "3.0.0"); // frontmatter 出现的字段优先
        assert_eq!(e.manifest.name, "acme"); // 未出现的字段沿用历史清单
        assert_eq!(e.manifest.category, "kb");
    }

    #[test]
    fn handles_zip_with_folder_root_and_agent_doc() {
        // 用户直接用系统工具把「agent-x」文件夹压成 zip：根路径是文件夹。
        let bytes = zip_stored(&[(
            "agent-x/AGENT.md",
            b"---\ndescription: An agent\n---\nbody",
        )]);
        let e = ok(&bytes);
        assert_eq!(e.manifest.category, "agent");
        assert_eq!(e.manifest.name, "agent-x");
        assert_eq!(e.manifest.description, "An agent");
        assert_eq!(archive::detect(&e.archive), Format::Zstd);
    }

    #[test]
    fn strips_nested_wrapper_folders() {
        let bytes = tar_zst(&[("outer/inner/SKILL.md", b"x")]);
        let e = ok(&bytes);
        assert_eq!(e.manifest.name, "inner");
        assert_eq!(e.file_count, 1);
    }

    #[test]
    fn no_strip_when_doc_already_at_root() {
        let bytes = tar_zst(&[("SKILL.md", b"x"), ("data/a.txt", b"y")]);
        let e = ok(&bytes);
        // 根目录已有 SKILL.md，不应剥离；name 无从推断，留空待用户填。
        assert_eq!(e.manifest.name, "");
        assert_eq!(e.file_count, 2);
    }

    #[test]
    fn rejects_when_doc_missing() {
        let bytes = tar_zst(&[("wrap/readme.txt", b"nope")]);
        assert!(extract_skill(&bytes).is_err());
    }

    #[test]
    fn ignores_junk_dirs() {
        let bytes = tar_zst(&[
            ("s/SKILL.md", b"x"),
            ("s/.git/config", b"junk"),
            ("s/node_modules/pkg/index.js", b"junk"),
        ]);
        let e = ok(&bytes);
        assert_eq!(e.file_count, 1); // 仅剩 SKILL.md
    }

    #[test]
    fn ignores_macos_appledouble_noise() {
        // 复刻 macOS `tar`/Finder 压缩注入的噪声：根部的 `._wrap`、`__MACOSX/`、`._SKILL.md`。
        // 这些若不在剥离根目录前剔除，会让唯一公共顶层目录判定失败。
        let bytes = tar_zst(&[
            ("._wrap", b"applenote"),
            ("wrap/._SKILL.md", b"applenote"),
            ("wrap/SKILL.md", b"# real"),
            ("__MACOSX/wrap/._SKILL.md", b"applenote"),
        ]);
        let e = ok(&bytes);
        assert_eq!(e.manifest.name, "wrap");
        assert_eq!(e.file_count, 1);
        assert!(e.skill_md.contains("real"));
    }

    #[test]
    fn rejects_unrecognized_format() {
        assert!(extract_skill(b"hello world, definitely not an archive").is_err());
    }
}

