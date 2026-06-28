------------------------------
## 🛸 Project Triskelion 1.0 Specification Manual

System Architecture, Protocols, and Implementation Guide for Coding Agents

------------------------------
## 📋 1. 项目概述 (Project Vision)
Triskelion 是一个面向 AI Agent 生态的分布式、多拓扑“工具网关、资源总线与开放市场平台”。项目名称致敬神盾局特工（Agent）总部大楼。
它将 Skills（Markdown 说明书）、MCPs（Model Context Protocol 服务）、CLIs（原生或脚本工具）以及 KBs（领域知识文件夹） 进行统一编排与解耦分发。
## 核心设计哲学

   1. GUI 服务人类，CLI 服务 AI：统一客户端 tsk 是 AI 的核心交互界面。
   2. 渐进式上下文加载：大模型平时只读取精简的 Skill 说明书，需要时才通过命令行（CLI）按需触发底层 MCP 或工具，从而将 Token 成本降低 90% 以上。
   3. 去中心化与高兼容：数据可全量导入导出，平台不强绑定特定客户端，对外提供标准开放 API。

------------------------------
## 🛠 2. 技术选型 (Technology Stack)

* 开发语言：Rust (全栈)
* 客户端 (CLI tsk) 核心库：
   * clap (v4)：高性能命令行参数解析
   * tokio：异步运行时
   * reqwest：异步 HTTP & SSE 客户端
   * keyring：跨平台调用系统原生凭据管理器（Mac Keychain / Windows Credential Manager）
   * serde & serde_json：序列化与反序列化
* 服务端 (Hub Server) 核心库：
   * axum (或 actix-web)：高性能多租户 Web 框架
   * tokio-stream：处理 SSE (Server-Sent Events) 流式路由
   * sqlx：带编译期检查的异步数据库驱动（搭配 PostgreSQL 或 SQLite）
   * jsonwebtoken：处理 tsk login 签发的 JWT

------------------------------
## 📐 3. 核心拓扑与执行逻辑 (Core Topology)
Project Triskelion 必须支持三种不同的 MCP 工具执行拓扑，由统一客户端 tsk 拦截 Agent 命令后动态路由：

1. 本地执行 (Local)  : Local Agent ──> tsk (CLI) ──> [本地拉起 Stdio 进程] ──> 本地 OS
2. 云端托管 (Cloud)  : Local Agent ──> tsk (CLI) ──(HTTP)──> Hub Server ──> [云端沙箱/容器执行]
3. 远程代理 (Remote) : Local Agent ──> tsk (CLI) ──(HTTP)──> Hub Server (注入凭据) ──> 远程第三方服务器

## 统一凭据缝合机制 (Credential Stitching)
开源 MCP 服务（如 GitHub MCP）保持 100% 原生性。

   1. 用户通过 tsk auth bind 将第三方凭据加密托管在 Hub 服务端。
   2. 当走 Cloud 或 Remote 拓扑时，Hub 网关截获请求，在出站/拉起容器的一瞬间，动态将该用户的三方真实 Token 以环境变量或 Header 形式缝合进请求中，实现生态资产的零修改接入。

------------------------------
## 📦 4. 核心资产配置文件规范 (tsk-package.json)
平台上流通的“复合应用包”或“单原子资源包”必须包含以下规范文件：

{
  "name": "shield-tactical-suite",
  "version": "1.0.0",
  "description": "神盾局多功能战术套装，包含 GitHub 工具、开发知识库与特工技能说明书",
  "type": "composite", 
  "resources": {
    "skills": ["./skills/core.md", "./skills/advanced.md"],
    "cli": ["./cli/graphify.py"],
    "kb": ["./kb/intel_dir/"]
  },
  "mcp_config": {
    "name": "github-inspector",
    "topology": "remote", 
    "remote_url": "https://github-mcp.io", 
    "required_auths": [
      {
        "provider": "github",
        "inject_as": "env.GITHUB_TOKEN" 
      }
    ]
  }
}

------------------------------
## 💻 5. tsk 客户端 (CLI) 功能清单
请 Agent 严格按照以下命令树（Command Tree）使用 clap 实现客户端：
## 基础资源直接管理 (Primitive Resource Management)

* tsk skill [add|remove|list]：直接管理单份纯文本的 Skill 说明书。
* tsk mcp [add|remove|list]：直接添加、修改或查看单项 MCP 注册配置（支持 Stdio 路径或远程 URL）。
* tsk cli [add|remove|list]：直接管理本地独立的战术脚本工具。
* tsk kb [add|remove|list]：直接注册、解除挂载本地包含领域知识的文件夹。

## 包生命周期管理 (Package Manager)

* tsk init：在当前目录初始化一个标准的开发资产包目录与 tsk-package.json 模板。
* tsk search <keyword>：调用 Hub 的开放 API 检索市场上的复合或单一资源包。
* tsk install <package-name>：下载资产包并解压，将对应的 Skill 和配置注册进 ~/.tsk/ 统一管理目录。
* tsk publish：将当前目录打包为 .tar.gz 并发布到 Hub 市场。

## 身份认证与通用执行中心

* tsk login：调用系统浏览器进行身份认证，获得平台 JWT，并通过 keyring 库安全固化到操作系统的 Keychain / 凭据管理器中。
* tsk auth bind <provider>：引导用户在网页端绑定特定的三方鉴权（如 GitHub OAuth）。
* tsk run <package/resource-name> <command> [args...]：核心执行入口。
* 大模型 Agent 会频繁调用此指令。
   * tsk 根据配置执行 mcp2cli 参数转译，动态选择本地 Stdio、云端托管或远程代理流式通信，并将最终结果以干净的 Markdown 打印在终端供 Agent 读取。

------------------------------
## ☁️ 6. Triskelion Hub 服务端功能清单
请使用 Axum 作为 Web 框架，Tokio 作为异步底座，实现以下三大模块 API：
## A. 多拓扑网关与凭据注入模块 (Gateway & Auth API)

* /v1/auth/login & /v1/auth/callback：处理用户的登录、JWT 签发及刷新。
* /v1/vault/bind：加密托管多租户的第三方 Token。
* /v1/mcp/router (SSE Stream 接口)：
* 接收来自客户端的统一工具调用请求（Header 携带平台 JWT）。
   * 根据包的 mcp_config 进行多租户识别，从 Vault 捞出对应用户的真实三方 Token。
   * 若为 cloud 拓扑：动态作为环境变量拉起临时 MCP Docker 容器。
   * 若为 remote 拓扑：作为反向代理网关，将 Token 拼入 Header，转发给三方 URL，并以 SSE 流式将结果回传给客户端。

## B. 市场开放 API 模块 (Open Market API)

* /v1/packages/search?q=xxx：支持任何第三方客户端接入、搜索技能。
* /v1/packages/download/:name：返回包的元数据与下载流。
* /v1/registry/publish：处理开发者提交的资产包。服务端接收后自动解压，利用内部集成的轻量 LLM Pipeline 提取 JSON Schema 并为其自动润色、生成前置的 SKILL.md 指引。

## C. 全量资产导入与导出模块 (Data Migration API)

* /v1/system/export：管理员/用户一键调用，将名下所有注册的 Skill、MCP、CLI 元数据及非敏感配置文件完整导出为单个 backup.tar.gz 加密压缩包。
* /v1/system/import：支持在全新的 Hub 实例上无缝上传该包，一键平移恢复全部工具生态。

------------------------------
## 💻 7. 核心架构 Rust 代码骨架参考
请 Coding Agent 参考以下核心数据结构和执行网关的设计伪代码开始编码：
## 客户端：核心转译与路由核心 (src/runtime/mod.rs)

use serde::{Deserialize, Serialize};use std::process::{Command, Stdio};

#[derive(Serialize, Deserialize, Debug, Clone)]pub enum Topology {
    Local,
    Cloud,
    Remote,
}

#[derive(Serialize, Deserialize, Debug)]pub struct McpConfig {
    pub name: String,
    pub topology: Topology,
    pub remote_url: Option<String>,
}
pub struct TriskelionRuntime;
impl TriskelionRuntime {
    pub async fn execute_tool(config: McpConfig, command: &str, args: Vec<&str>) -> Result<String, Box<dyn std::error::Error>> {
        match config.topology {
            Topology.Local => {
                // 拓扑 1：在本地作为 Stdio 进程拉起原生 MCP 
                // 此处请 Coding Agent 实现具体的 JSON-RPC 读写管道逻辑
                let output = Command::new("node")
                    .arg("./mcp_server.js") // 示例路径
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .output()?;
                Ok(String::from_utf8(output.stdout)?)
            },
            Topology.Cloud | Topology.Remote => {
                // 拓扑 2 & 3：转译参数，通过 reqwest 异步网络流向 Hub 网关发起请求
                let client = reqwest::Client::new();
                let platform_jwt = "从本地keyring获取的JWT"; 
                
                let response = client.post("https://triskelion.ai")
                    .header("Authorization", format!("Bearer {}", platform_jwt))
                    .json(&serde_json::json!({
                        "mcp_name": config.name,
                        "command": command,
                        "arguments": args
                    }))
                    .send()
                    .await?
                    .text()
                    .await?;
                Ok(response)
            }
        }
    }
}

## 服务端：网关凭据缝合与代理转发 (src/gateway/router.rs)

use axum::{Extension, Json, response::IntoResponse};use serde_json::Value;
pub async fn handle_mcp_routing(
    Extension(user_id): Extension<String>, // 从 JWT 中解出的用户身份
    Json(payload): Json<Value>
) -> impl IntoResponse {
    let mcp_name = payload["mcp_name"].as_str().unwrap();
    
    // 1. 从数据库/Vault 中查询当前用户针对该 MCP 绑定的第三方真实凭据
    let third_party_token = fetch_user_secret_from_vault(&user_id, mcp_name).await;
    
    // 2. 根据资产包声明的拓扑类型进行处理
    // 如果是 Remote 拓扑，作为反向代理，缝合凭据后转发给真实的第三方服务器
    let remote_target_url = "https://thirdparty-mcp.com"; 
    
    let client = reqwest::Client::new();
    let sse_stream = client.post(remote_target_url)
        .header("X-ThirdParty-Authorization", format!("Bearer {}", third_party_token)) // 核心：凭据注入
        .json(&payload)
        .send()
        .await;

    // 3. 将网络流转换为 Axum 的 Streamable SSE 响应，实时吐回给本地 tsk 客户端
    // (请 Coding Agent 补全具体的 Async Stream 转换逻辑)
    "SSE Stream Output"
}
async fn fetch_user_secret_from_vault(user_id: &str, mcp_name: &str) -> String {
    // 实际应查询加密数据库，此处返回模拟 Token
    "real_github_token_xyz123".to_string()
}

------------------------------
## 🚀 8. Agent 启动指令 (Instructions for Agent)

💡 给 Coding Agent 的提示：
"请通读上述规范文档。我们将采用渐进式开发。
第一步：请先在客户端项目中，使用 clap 完成基础资源直接管理命令树（Skill、MCP、CLI、KB 的 add/remove/list）的骨架搭建，并定义好 ~/.tsk/ 目录的本地初始化和元数据存储逻辑。
做好准备后，请输出你设计的客户端基础数据结构，并引导我开始进行代码审查。"


