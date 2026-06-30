//! 构建脚本：在编译 server 侧前自动构建前端（web/ → web/dist），
//! 其产物由 `rust-embed` 编译进 `triskelion` 二进制。
//!
//! 策略（务求稳健，绝不因前端环境缺失而阻断 `cargo build`）：
//! - 始终保证 `web/dist` 目录存在（rust-embed 要求嵌入目录存在）；
//! - 仅当 `bun` 可用且 `web/node_modules` 已安装时，才运行 `bun run build`；
//! - 失败只告警，不 panic（此时沿用已有 dist 产物或空目录）。
//!
//! 首次准备前端依赖：`cd web && bun install`（本环境受 TLS 拦截，
//! 需 `NODE_TLS_REJECT_UNAUTHORIZED=0 bun install --registry https://registry.npmmirror.com`）。

use std::path::Path;
use std::process::Command;

fn main() {
    // 仅在构建 server 侧时关心前端。
    if std::env::var("CARGO_FEATURE_SERVER").is_err() {
        return;
    }

    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let web = Path::new(&manifest).join("web");
    let dist = web.join("dist");

    // 源码变化时重跑。
    println!("cargo:rerun-if-changed=web/src");
    println!("cargo:rerun-if-changed=web/index.html");
    println!("cargo:rerun-if-changed=web/package.json");
    println!("cargo:rerun-if-changed=web/vite.config.ts");
    println!("cargo:rerun-if-env-changed=TRISKELION_SKIP_WEB_BUILD");

    // 保证嵌入目录存在。
    let _ = std::fs::create_dir_all(&dist);

    if std::env::var("TRISKELION_SKIP_WEB_BUILD").is_ok() {
        println!("cargo:warning=TRISKELION_SKIP_WEB_BUILD set, skip frontend build");
        ensure_placeholder(&dist);
        return;
    }

    let has_bun = Command::new("bun").arg("--version").output().is_ok();
    let has_modules = web.join("node_modules").is_dir();

    if !web.join("package.json").exists() {
        // 发布包场景：仅随包携带预构建的 web/dist，无前端源码。
        note_prebuilt_or_placeholder(&dist);
        return;
    }
    if !has_bun || !has_modules {
        // 无法（重新）构建前端。若已随包携带预构建产物则直接使用，否则放占位页。
        if has_prebuilt(&dist) {
            println!("cargo:warning=使用随包预构建的 web/dist（bun={has_bun}, node_modules={has_modules}），跳过前端构建");
        } else {
            println!(
                "cargo:warning=跳过前端构建（bun={has_bun}, node_modules={has_modules}）。\
                 如需内置 UI：cd web && bun install && bun run build"
            );
        }
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
