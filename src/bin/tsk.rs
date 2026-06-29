//! `tsk` —— Triskelion 客户端 CLI。
//!
//! 登录与渐进式凭据缝合、MCP 注册、变量管理，以及内置 mcp2cli 动态转译的
//! `tsk run`：把任意 MCP 当命令行使用，运行时自动从 Hub 拉取并注入用户变量。
//!
//! 详见 `docs/design.md` §5.1。

fn main() {
    if let Err(e) = triskelion::client::run() {
        eprintln!("tsk: {e:#}");
        std::process::exit(1);
    }
}
