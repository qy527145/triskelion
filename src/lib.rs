//! Triskelion library root.
//!
//! 两个二进制通过 Cargo features 复用本 crate：
//! - `client` feature —— `tsk` 客户端 CLI 侧（`client::*`）
//! - `server` feature —— `triskelion` Hub 服务端侧（`server::*`）
//! - 共享层（wire types、manifest schema、占位符插值等）始终构建
//!
//! 详见 `docs/design.md`。

pub mod shared;

/// 压缩体格式探测与压缩参数（zstd 升级 + gzip 向后兼容），client / server 双侧复用。
#[cfg(any(feature = "client", feature = "server"))]
pub mod archive;

/// MCP 传输层，client / server 双侧按需复用。
#[cfg(any(feature = "client", feature = "server"))]
pub mod mcp;

#[cfg(feature = "client")]
pub mod client;

#[cfg(feature = "server")]
pub mod server;
