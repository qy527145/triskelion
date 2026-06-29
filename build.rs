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
        ensure_placeholder(&dist);
        return;
    }
    if !has_bun || !has_modules {
        println!(
            "cargo:warning=跳过前端构建（bun={has_bun}, node_modules={has_modules}）。\
             如需内置 UI：cd web && bun install && bun run build"
        );
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
