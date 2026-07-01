//! 构建脚本：在编译 server 侧前按需构建前端（web/ → web/dist），
//! 其产物由 `rust-embed` 编译进 `triskelion` 二进制。
//!
//! 策略（务求稳健，绝不因前端环境缺失而阻断 `cargo build`）：
//! - 始终保证 `web/dist` 目录存在（rust-embed 要求嵌入目录存在）；
//! - **自动准备依赖**：`bun` 可用但 `web/node_modules` 缺失时，自动 `bun install`
//!   （无需手动 `cd web && bun install`）；受限网络见下方 env 说明；
//! - **按时间戳增量**：仅当前端源码（web/src、index.html、package.json、
//!   vite.config.ts、tsconfig.json、bun.lock 等）比 `web/dist` 产物更新时，
//!   才运行 `bun run build`；前端未变则直接沿用已提交的 dist，工作区保持干净、
//!   不会因每次 `cargo build` 重新生成带哈希的产物而变脏；
//! - 覆盖开关：`TRISKELION_BUILD_WEB=1` 强制重建，`TRISKELION_SKIP_WEB_BUILD=1` 强制跳过；
//! - 失败只告警，不 panic（此时沿用已有 dist 产物或空目录）。
//!
//! 受限网络（TLS 拦截 / 需镜像源）下让自动安装生效：
//!   TRISKELION_BUN_INSTALL_ARGS="--registry https://registry.npmmirror.com" \
//!   NODE_TLS_REJECT_UNAUTHORIZED=0 cargo build
//! 也可写进 `.cargo/config.toml` 的 `[env]` 一劳永逸（见项目 README）。

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
    println!("cargo:rerun-if-env-changed=TRISKELION_BUN_INSTALL_ARGS");

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

    let has_bun = Command::new("bun").arg("--version").output().is_ok();
    if !has_bun {
        // 没有 bun：无法安装依赖也无法构建。有预构建产物则直接用，否则占位页。
        if has_prebuilt(&dist) {
            println!("cargo:warning=未检测到 bun，使用已有 web/dist，跳过前端构建");
        } else {
            println!("cargo:warning=未检测到 bun，跳过前端构建。如需内置 UI：安装 bun 后重新编译");
        }
        ensure_placeholder(&dist);
        return;
    }

    // 依赖缺失则自动 `bun install`（不再要求用户手动准备 node_modules）。
    if !web.join("node_modules").is_dir() && !ensure_deps(&web) {
        // 安装失败：退回已有产物或占位页，仍不阻断 cargo build。
        if has_prebuilt(&dist) {
            println!("cargo:warning=bun install 失败，使用已有 web/dist，跳过前端构建");
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

/// 确保前端依赖就绪：`node_modules` 缺失时自动 `bun install`。
/// 受限网络（TLS 拦截 / 需镜像源）可通过环境变量注入额外参数，例如：
///   TRISKELION_BUN_INSTALL_ARGS="--registry https://registry.npmmirror.com"
///   NODE_TLS_REJECT_UNAUTHORIZED=0
/// 子进程继承本进程环境，故上述 env 直接透传给 bun。返回是否安装成功。
fn ensure_deps(web: &Path) -> bool {
    println!("cargo:warning=web/node_modules 缺失，运行 `bun install`…");
    let mut cmd = Command::new("bun");
    cmd.arg("install").current_dir(web);
    if let Ok(extra) = std::env::var("TRISKELION_BUN_INSTALL_ARGS") {
        cmd.args(extra.split_whitespace());
    }
    match cmd.status() {
        Ok(s) if s.success() => web.join("node_modules").is_dir(),
        Ok(s) => {
            println!(
                "cargo:warning=bun install 失败（exit {s}）。受限网络可设置 \
                 TRISKELION_BUN_INSTALL_ARGS=\"--registry https://registry.npmmirror.com\" \
                 及 NODE_TLS_REJECT_UNAUTHORIZED=0 后重试"
            );
            false
        }
        Err(e) => {
            println!("cargo:warning=无法运行 bun install（{e}）");
            false
        }
    }
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
