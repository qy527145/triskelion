------------------------------
## 🛸 Project Triskelion 1.0 System Architecture Specification

For Senior Software Engineering Agents (LLMs). Follow this specification strictly to design and implement the system.

------------------------------
## 📋 1. 项目背景与大一统世界观 (Project Vision & Philosophy)## 1.1 背景隐喻
Triskelion（名称致敬《神盾局特工》总部大楼）是一个面向大模型（Agent）工具生态的去中心化、多拓扑“统一资源网关与开放市场平台”。

* 在 AI 时代，“GUI 服务人类，CLI 服务 AI”。大模型通过阅读轻量化的工具说明书，像黑客一样自主在终端（Terminal）呼叫命令行（CLI）来改变世界。

## 1.2 核心痛点与终极工程减法（局长关键纠正）

   1. 干掉 Token 膨胀与认知过载：原生 MCP（Model Context Protocol）协议会把满载的 JSON Schema 强行塞满 Agent 的上下文。Triskelion 采用渐进式上下文加载：Agent 初始化时仅加载几百 Token 的极其精简的 SKILL.md（能力说明书）。当且仅当需要执行动作时，才通过 CLI 按需触发，降维打击 Token 消耗，成功率提升至 99%。
   2. 大一统的数据实体模型（万物皆 Skill）：
   * 在 Triskelion 的业务世界里，“知识库（Knowledge Base）”和“工具链（Toolchain）”不是独立的数据实体，它们仅仅是逻辑层面的分类标签。
      * 本质上，平台里流转的一切基础资产，全都是 Skill。一个 Skill 包可以是一份“纯文本的裸说明书”（能力内置），也可以是一份“依赖了底层 MCP 工具的能力说明书”。
   3. 无沙箱/无容器的极致性能：
   * 彻底放弃 Docker、K8s 等沉重的容器隔离层。
      * 系统完全基于 Rust 异步底层 重新设计。将复杂的环境运维隔离，转化为极其轻量、高频并发的 “本地进程管理” 与 “云端异步网络反向代理网关”。
   
------------------------------
## 📐 2. 三种 MCP 运行时拓扑的推敲与设计 (Multi-Topology Runtime)
统一客户端 tsk 拦截 Agent 的命令行呼叫后，会根据包中定义的 mcp_config，无缝调度以下三种拓扑之一：
## 2.1 本地执行模式 (Local)

* 适用场景：本地文件修改、本地私有数据库运维。
* 执行逻辑：用户执行 tsk install 时将包的源码下载到本地。运行时，本地的 tsk 客户端（Rust 编写）直接调用操作系统的 std::process::Command，在用户本地系统拉起该 MCP 的 Stdio 进程（如 Node.js 或 Python 脚本），通过标准输入输出进行内存级 JSON-RPC 通信。

## 2.2 云端托管模式 (Cloud)

* 适用场景：公共高频工具（如天气、通用网络爬虫），开发者不想管服务器。
* 执行逻辑（无沙箱 Serverless 玩法）：开发者上传源码到 Hub 后，Hub 服务器直接作为一个长期运行的常驻 FaaS 服务拉起该 MCP 脚本。当不同用户Phil调用该工具时，Hub 采用“无状态（Stateless）单例设计”，在内存请求中动态带入该用户的特定 Context，秒级响应（压入 5 毫秒以内）。

## 2.3 远程代理模式 (Remote Proxy)

* 适用场景：大型 SaaS 公司的商业 MCP 服务（如 GitHub、Linear 官方自带的 SSE/HTTP 远程集群），其源码对平台保密。
* 执行逻辑：Hub 服务器不运行任何第三方代码，仅扮演一个“智能网络过滤器 / 反向代理”。Hub 接收到客户端请求 $\rightarrow$ 动态缝合该用户的真实凭据 $\rightarrow$ 转发给三方的 remote_url $\rightarrow$ 三方返回 SSE (Server-Sent Events) 数据流 $\rightarrow$ Hub 网关流式原样吐回给本地 tsk。Hub 彻底免除安全责任，100% 零修改吞下现有开源远程 MCP 生态。

------------------------------
## 🔒 3. 渐进式多租户凭据缝合机制 (Lazy Authentication)
为了让不同用户在调用同一个云端或远程工具时，拿到的信息完全不同，系统引入声明式凭据缝合（Credential Stitching）：

   1. 默认无鉴权启动：系统初始状态下没有任何用户的私密 Key，极速运行。
   2. 按需触发设置：Phil 的 Agent 在阅读说明书时，发现执行该技能必须提供鉴权信息。用户在本地敲击指令：
   
   tsk auth set <package-name> GITHUB_TOKEN "phil_secret_xyz123"
   
   3. 凭据托管：tsk 客户端通过 keyring 库固化在本地系统凭据库中，并安全同步到 Hub 平台的加密凭据池（使用 AES-256-GCM 算法加密）。
   4. 运行时缝合（仅限 Cloud / Remote 拓扑）：当 Agent 呼叫工具时，Header 携带平台统一登录（tsk login）的 JWT。Hub 网关拦截请求 $\rightarrow$ 校验 JWT 锁定用户 Phil $\rightarrow$ 从云端保险箱捞出 Phil 的真实 GitHub Token $\rightarrow$ 在出站/转发网络请求的一瞬间，动态将其缝合进 HTTP Header 或容器环境变量中。

------------------------------
## 🗄 4. 数据实体与配置文件规范 (tsk-package.json)
请 Agent 严格按照以下“多态统一”的数据结构来设计数据库 Schema 和包配置文件：

{
  "name": "shield-development-pack",
  "version": "1.0.0",
  "category": "toolchain", // 逻辑分类：skill (技能) / kb (知识库) / toolchain (工具链)。底层全由 skill 数据结构统一承载
  "visibility": "private", // 可见级别：默认为 private (仅自己可见/调试)，开发者点击发布后变为 public (对所有人可见)
  "resources": {
    "skills": ["./skills/manual.md"] // Skill 资源。可以完全不依赖下面的 mcp_config，作为一个“裸说明书包”
  },
  "mcp_config": {
    "name": "github-inspector",
    "topology": "remote", // 核心拓扑声明：local | cloud | remote
    "remote_url": "https://github-mcp.io", // 仅在 topology 为 remote 时生效
    "required_auths": [
      {
        "provider": "github",
        "inject_as": "env.GITHUB_TOKEN" // 声明需要绑定的鉴权信息和注入位置
      }
    ]
  }
}

------------------------------
## 🔄 5. 全生命周期核心动作功能树 (The Master Command Tree)
请 Agent 采用全栈 Rust 实现以下两端的功能体系，并支持全量资源的导入与导出 (import / export) 方便数据完美平移与去中心化私有部署。
## 5.1 tsk 客户端 (CLI) 功能矩阵

* 基础资源直接操控力：
* tsk skill [add|remove|list]：直接直接增删改查单份纯文本的 Skill 说明书。
   * tsk mcp [add|remove|list]：直接管理底层 MCP 注册信息。
   * tsk cli [add|remove|list] / tsk kb [add|remove|list]
* 开发测试与版本纠错（随时重新发布）：
* tsk init：在本地当前目录生成配置文件模板。
   * tsk run <package> <command> [args...]：核心入口。内置 mcp2cli 动态参数转译逻辑。开发者本地调通后，随时可以修改代码或说明书，重复执行 tsk publish 覆盖并热更新 Hub 资产。
* 使用者消费链：
* tsk login / tsk auth bind / tsk auth set：处理统一平台登录与渐进式凭据绑定。
   * tsk search / tsk install：进行轻量级技能下载，保持本地操作系统绝对干净。

## 5.2 Triskelion Hub (Web Server) 功能矩阵

* 多拓扑异步反向代理网关：基于 Axum + Tokio-stream。负责处理 mcp/router SSE 流式路由请求，高效执行凭据缝合与数据异步吐回。
* 去中心化数据平移：
* POST /v1/system/export：一键导出名下全量 Skill 文本、MCP 拓扑配置及非敏感文件为单个 backup.tar.gz。
   * POST /v1/system/import：支持在全新部署的 Hub 上无缝恢复。
* 开放 API 矩阵：提供完全开放的 RESTful / SSE 标准 API，将 Hub 作为底层纯净的市场与路由总线，以便 Cursor、Claude Code、Dify 等市面上任何现有的第三方客户端轻松接入调用。

------------------------------
## 🤖 6. 给 Coding Agent 的开发启动指令 (Implementation Orders)

🚀 Maximum Priority Directive to Agent:

   1. You understand that this project uses NO containers (NO Docker). Everything is based on native local sub-process execution (for local topology) and async HTTP/SSE reverse proxying (for cloud/remote topologies).
   2. You understand that "Knowledge Base" and "Toolchain" are just tags; under the hood, everything is a unified Skill model.

Your Next Steps:

* Step 1: Design the SQLite/PostgreSQL database migration schema for the Hub Server that perfectly aligns with the tsk-package.json specification (including visibility, categories, topologies, and package binary storage).
* Step 2: Provide the implementation layout for the Client CLI's tsk run argument interceptor using clap v4, showing how you plan to dynamically parse parameters before converting them into MCP JSON-RPC payloads.

Start by generating the DB schema and layout now.

------------------------------
## 🌀 Triskelion 全生命周期大一统动作图谱 (The Grand Lifecycle)
【1. 筹备期】 Tony 获取 tsk ──> 登录/绑定 Hub ──> 本地编写原生 MCP 
                                                        │
                                                        ▼
【2. 本地测试】 运行 mcp2cli ──> 动态生成本地 CLI ──> 配合本地 Skill ──> Agent 测试成功
                                                        │
                                                        ▼
【3. 打包发布】 tsk init ──> 声明配置(可见性/依赖/鉴权) ──> 声明逻辑分类 ──> tsk publish
                                                        │
                                                        ▼
【4. Hub 托管】 平台解析 ──> 更新/多拓扑布署 ──> 建立逻辑索引(技能/知识库/工具链)
                                                        │
                                                        ▼
【5. 发现安装】 Phil 的 Agent 触发任务 ──> tsk search ──> tsk install 
                                                        │
                                                        ▼
【6. 渐进鉴权】 读说明书提示 ──> 触发鉴权需求 ──> tsk auth set ──> 固化凭据
                                                        │
                                                        ▼
【7. 运行时】   Agent 执行命令 ──> 本地 mcp2cli 转译 ──> Hub 网关动态注入 Token ──> 路由执行