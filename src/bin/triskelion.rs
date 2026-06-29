//! `triskelion` —— Triskelion Hub Web Server。
//!
//! 多拓扑异步反向代理网关：用户名密码鉴权（JWT）、MCP 注册表、AES-256-GCM
//! 加密凭据池，以及 `tsk run` 的渐进式凭据缝合解析接口。
//!
//! 详见 `docs/design.md` §5.2。

fn main() -> anyhow::Result<()> {
    triskelion::server::run()
}
