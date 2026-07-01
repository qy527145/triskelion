//! 构建脚本：在编译 server 侧前按需构建前端（web/ → web/dist），
//! 其产物由 `rust-embed` 编译进 `triskelion` 二进制。
//!
//! 策略（务求稳健，绝不因前端环境缺失而阻断 `cargo build`）：
//! - 始终保证 `web/dist` 目录存在（rust-embed 要求嵌入目录存在）；
//! - **永不 `bun install`**：仅当 `bun` 命令存在且 `web/node_modules` 已就绪时，
//!   才可能运行 `bun run build`。因此任何 `cargo install`（crates.io 或 --git，
//!   都不含 node_modules）必然直接沿用随包提交的 `web/dist`，终端用户零 bun 依赖、
//!   零网络、零意外构建；
//! - **按时间戳增量**：即便可构建，也仅当前端源码（web/src、index.html、
//!   package.json、vite.config.ts、tsconfig.json、bun.lock 等）比 `web/dist` 产物更新时
//!   才 `bun run build`；未变则跳过，工作区保持干净；
//! - 覆盖开关：`TRISKELION_BUILD_WEB=1` 强制重建，`TRISKELION_SKIP_WEB_BUILD=1` 强制跳过；
//! - 失败只告警，不 panic（此时沿用已有 dist 产物或空目录）。
//!
//! 开发者首次准备前端依赖：`cd web && bun install`（本环境受 TLS 拦截，
//! 需 `NODE_TLS_REJECT_UNAUTHORIZED=0 bun install --registry https://registry.npmmirror.com`）。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

fn main() {
    // 仅在构建 server 侧时关心前端。
    if std::env::var("CARGO_FEATURE_SERVER").is_err() {
        return;
    }

    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let root = Path::new(&manifest);
    let web = root.join("web");
    let dist = web.join("dist");

    // 源码变化时重跑（仅声明存在的路径：cargo 对不存在的 rerun-if-changed 路径会退化为每次都重跑）。
    for rel in [
        "web/src",
        "web/index.html",
        "web/package.json",
        "web/vite.config.ts",
        "web/tsconfig.json",
        "web/bun.lock",
    ] {
        if root.join(rel).exists() {
            println!("cargo:rerun-if-changed={rel}");
        }
    }
    // web/dist 是 rust-embed 的嵌入目录。这里**无条件**声明它为重跑触发：
    // 一旦它被删除（cargo 视缺失路径为“已变化”），本脚本会重跑并经 create_dir_all
    // 重建之，避免 rust-embed 编译期报 “folder does not exist”。目录存在时正常按变化跟踪。
    println!("cargo:rerun-if-changed=web/dist");
    println!("cargo:rerun-if-env-changed=TRISKELION_BUILD_WEB");
    println!("cargo:rerun-if-env-changed=TRISKELION_SKIP_WEB_BUILD");

    // 保证嵌入目录存在。
    let _ = std::fs::create_dir_all(&dist);

    // 强制跳过。
    if std::env::var("TRISKELION_SKIP_WEB_BUILD").is_ok() {
        println!("cargo:warning=TRISKELION_SKIP_WEB_BUILD set, skip frontend build");
        note_prebuilt_or_placeholder(&dist);
        return;
    }

    if !web.join("package.json").exists() {
        // 发布包场景：仅随包携带预构建的 web/dist，无前端源码。
        note_prebuilt_or_placeholder(&dist);
        return;
    }

    // 永不 `bun install`：仅当 bun 命令存在且 node_modules 已就绪时才可能重建前端。
    // 任何 `cargo install`（crates.io 或 --git）都不含 node_modules，故必然直接
    // 沿用随包提交的 web/dist，绝不触发前端构建——保证终端用户零 bun 依赖、零网络。
    let has_bun = Command::new("bun").arg("--version").output().is_ok();
    let has_modules = web.join("node_modules").is_dir();
    if !has_bun || !has_modules {
        if has_prebuilt(&dist) {
            println!("cargo:warning=使用已有 web/dist（bun={has_bun}, node_modules={has_modules}），跳过前端构建");
        } else {
            println!(
                "cargo:warning=跳过前端构建（bun={has_bun}, node_modules={has_modules}）。\
                 如需重建内置 UI：cd web && bun install，然后重新编译"
            );
        }
        ensure_placeholder(&dist);
        return;
    }

    // 增量判定：前端源码未比 dist 产物更新，且非强制重建 → 跳过（工作区不变脏）。
    let forced = std::env::var("TRISKELION_BUILD_WEB").is_ok();
    if !forced && !web_dist_is_stale(&web, &dist) {
        println!("cargo:warning=web/dist 已是最新（前端源码未变），跳过前端构建");
        ensure_placeholder(&dist);
        return;
    }

    println!("cargo:warning=running `bun run build` for web UI…");
    let status = Command::new("bun")
        .args(["run", "build"])
        .current_dir(&web)
        .status();
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => println!("cargo:warning=前端构建失败（exit {s}），沿用已有 dist 产物"),
        Err(e) => println!("cargo:warning=无法运行 bun（{e}），沿用已有 dist 产物"),
    }
    ensure_placeholder(&dist);
}

/// 前端产物是否「过期」：任一源码文件的 mtime 晚于 dist 产物中最早的 mtime。
/// 无预构建产物则视为过期（需要首次构建）。取不到时间戳时保守地判为过期。
fn web_dist_is_stale(web: &Path, dist: &Path) -> bool {
    if !has_prebuilt(dist) {
        return true;
    }
    let sources = [
        web.join("src"),
        web.join("index.html"),
        web.join("package.json"),
        web.join("vite.config.ts"),
        web.join("tsconfig.json"),
        web.join("bun.lock"),
        web.join("bun.lockb"),
    ];
    let outputs = [dist.join("index.html"), dist.join("assets")];
    match (newest_mtime(&sources), oldest_mtime(&outputs)) {
        (Some(src), Some(out)) => src > out,
        _ => true,
    }
}

/// 一组路径（目录递归 + 目录自身）的最新修改时间。
fn newest_mtime(paths: &[PathBuf]) -> Option<SystemTime> {
    paths.iter().filter_map(|p| extremum_mtime(p, true)).max()
}

/// 一组路径（目录递归 + 目录自身）的最早修改时间。
fn oldest_mtime(paths: &[PathBuf]) -> Option<SystemTime> {
    paths.iter().filter_map(|p| extremum_mtime(p, false)).min()
}

/// 递归求某路径下的极值 mtime（newest=true 取最大，否则取最小）。
/// 目录会连同自身 mtime 一并纳入，以捕获文件的新增/删除。
fn extremum_mtime(path: &Path, newest: bool) -> Option<SystemTime> {
    let meta = std::fs::metadata(path).ok()?;
    let mut acc = meta.modified().ok();
    if meta.is_dir() {
        if let Ok(entries) = std::fs::read_dir(path) {
            for e in entries.flatten() {
                if let Some(t) = extremum_mtime(&e.path(), newest) {
                    acc = Some(match acc {
                        Some(a) if (newest && a >= t) || (!newest && a <= t) => a,
                        _ => t,
                    });
                }
            }
        }
    }
    acc
}

/// 若 dist 为空（既无构建产物又无 index.html），放一个占位页，避免运行期空白。
fn ensure_placeholder(dist: &Path) {
    if dist.join("index.html").exists() {
        return;
    }
    let _ = std::fs::write(
        dist.join("index.html"),
        "<!doctype html><meta charset=utf-8><title>Triskelion</title>\
         <body style=\"font-family:sans-serif;padding:40px\">\
         <h1>Triskelion Hub</h1>\
         <p>Web UI 尚未构建。请运行 <code>cd web &amp;&amp; bun install &amp;&amp; bun run build</code> 后重新编译。</p>\
         <p>API 仍正常工作，见 <code>/v1/*</code>。</p></body>",
    );
}

/// 是否存在「真实」的预构建前端产物（vite 会生成 assets/ 目录），用以区分占位页。
fn has_prebuilt(dist: &Path) -> bool {
    dist.join("index.html").exists() && dist.join("assets").is_dir()
}

/// 发布包安装场景：有预构建产物则告知直接使用，否则放占位页。
fn note_prebuilt_or_placeholder(dist: &Path) {
    if has_prebuilt(dist) {
        println!("cargo:warning=使用随包预构建的 web/dist，跳过前端构建");
    }
    ensure_placeholder(dist);
}
