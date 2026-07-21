# triskelion

Welcome to the Triskelion. Equip your AI Agents with tactical Skills, localized Knowledge Intel, and bulletproof CLI tools.

万物皆 **Skill**：技能（skill）、知识库（kb）、工具链（toolchain）、Agent（agent）只是逻辑分类标签，底层共用同一数据结构。Agent 初始化时只读几百 Token 的精简说明书（`agent` 分类用 `AGENT.md`，其余用 `SKILL.md`），需要动作时才用 `tsk` 触发底层 MCP。

## 两个市场

- **技能市场（Skill）** —— 平台首页。技能包是一个文件夹（必须含说明书：`agent` 分类为 `AGENT.md`，其余为 `SKILL.md`），发布前由 `tsk build` 打成 `tar.zst`。服务端只持元数据 + 说明书文本，庞大的数据体以压缩包形式按 sha256 内容寻址承载。支持按分类、标签、关键字检索。
- **MCP 市场** —— 注册可被 `tsk run` 当 CLI 调用的 MCP 工具，声明运行拓扑（local / remote）与所需 `{VAR}` 变量。

## 技能全生命周期（tsk）

```bash
# 1) 脚手架：生成带 frontmatter 元数据的 SKILL.md（Agent 用 tsk skill init --category agent，生成 AGENT.md）
tsk skill init my-skill && cd my-skill

# 2) 本地打包校验（产出 .tsk/dist/<name>-<version>.tar.zst）
tsk build                       # 等价于 tsk skill build

# 3) 发布（build + 上传元数据/SKILL.md + 上传压缩体）
tsk skill publish --visibility public

# 4) 发现与拉取
tsk skill search "" --category toolchain --tag github
tsk pull alice/shield-dev-pack        # 下载、校验 sha256、解压到 ./shield-dev-pack（默认最新版）
tsk pull alice/shield-dev-pack@1.0.0  # 拉取指定版本（服务端保留全部已发布版本）
tsk skill versions alice/shield-dev-pack  # 列出全部已发布版本
tsk skill show alice/shield-dev-pack  # 查看元数据 + SKILL.md（也支持 @version）
tsk skill list                        # 名下全部技能（含私有）
```

> 版本语义：每次 `publish` 都会保留该版本的完整副本；**重复发布同一版本号会覆盖该版本**；
> 发布低于当前最新版的旧版本号只更新该版本副本，不影响「最新版」。

### 元数据：SKILL.md frontmatter

技能元数据直接写在说明书头部的 YAML frontmatter 里（与 Claude Code 技能格式同构，无需单独的清单文件）：

```markdown
---
name: shield-dev-pack
version: 1.0.0
category: toolchain                 # skill | kb | toolchain | agent
description: 盾局开发工具链
tags: [github, ci]
mcp_dependencies: [alice/github-inspector]        # 依赖的底层 MCP
preferred_tools: [github-inspector/create_issue]  # 倾向优先使用的工具
---

# 盾局开发工具链
……正文即 Agent 阅读的能力说明书……
```

- **外部导入的技能缺字段时自动用默认值**：`name` ← 文件夹名，`version` ← `0.1.0`，
  `category` ← 按说明书推断（仅有 AGENT.md 归为 agent，否则 skill），标签为空。
  第三方生态的附加字段（如 `allowed-tools`）原样保留、不受影响。
- 列表字段支持内联数组 `[a, b]`、块列表（`- item`）或逗号分隔标量（`tags: a, b`）三种写法。
- 历史 `tsk-skill.json` 仍被兼容读取（作为缺省值来源），frontmatter 里出现的字段优先。

技能若依赖底层 MCP，在 `SKILL.md` 里用 `tsk` 包装调用即可（说明书会随 `tsk pull` 一并提示）：

```bash
tsk run alice/github-inspector --help
tsk run alice/github-inspector create_issue --title "..." --body "..."
```

纯文本「裸说明书」技能无需任何压缩体，也可在 Web 端「我的技能 → 新建技能」直接创建。

### 开放 API：第三方客户端查询版本、安装指定版本

Hub 的 REST API 完全开放（CORS 放行跨源），任意客户端可直接对接。公开资源匿名可读；
私有资源带 `Authorization: Bearer <token>`（`POST /v1/auth/login` 以 `{"username","password"}` 换取）。

```bash
HUB=http://127.0.0.1:8787

# 1) 列出某技能的全部已发布版本（新→旧）
curl $HUB/v1/skill/alice/shield-dev-pack/versions
# → [{"version":"1.1.0","archive_sha256":"…","archive_size":1234,"created_at":"…"},
#    {"version":"1.0.0", …}]

# 2) 查询指定版本的详情（说明书/依赖/压缩体指针；省略 ?version= 为最新版）
curl "$HUB/v1/skill/alice/shield-dev-pack?version=1.0.0"
# 响应含 versions 字段（全部版本号，新→旧），可一次拿到「详情 + 可用版本」

# 3) 下载指定版本的压缩体（tar.zst；校验响应体 sha256 == 详情的 archive_sha256）
curl -LO "$HUB/v1/skill/alice/shield-dev-pack/archive?version=1.0.0"
```

Web 端技能详情弹窗内同样可浏览版本历史、查看/下载任意历史版本。

## 免登录使用与本地变量

`tsk` 对登录**非强制**：未登录即可浏览公开市场、拉取公开技能、`tsk run` 公开 MCP。

- **变量（凭据）**：`tsk secret set/list/rm` 未登录时读写本地文件 `~/.tsk/secrets.json`（0600）；
  登录后写变量会在本地之外**同时写线上**（个人变量）。运行解析时**本地优先于线上**
  （同名变量本地覆盖线上）。
- **Hub 地址**：未登录可用 `TRISKELION_HUB` 环境变量指定 Hub，例如
  `TRISKELION_HUB=http://hub tsk run alice/foo ...`（带 `/` 的 `owner/name` 形式）。

## 数据目录

- **服务端**：默认 `~/.triskelion`，可用 `TRISKELION_SERVER_DATA_DIR` 覆盖（兼容旧的 `TRISKELION_DATA_DIR`）。
  目录布局：`keys/`（`jwt_rsa.pem` RS256 签发密钥、`master.key` 凭据池主密钥）、
  `data/`（`blobs/` 技能压缩体按 sha256 内容寻址、`hub.db` 默认 SQLite 库）。
- **客户端 CLI**：统一存放于 `~/.tsk`（`config.json` 登录态 + `secrets.json` 本地变量），可用 `TRISKELION_CLIENT_DATA_DIR` 覆盖。

## 数据库（SQLite 默认 / PostgreSQL / MySQL）

Hub 的持久化默认使用 **SQLite**（`<数据目录>/data/hub.db`，零配置开箱即用，适合单机与开发）。
生产环境可通过 `TRISKELION_DATABASE_URL` 切换到外部数据库，按连接串前缀自动选择后端：

```bash
# PostgreSQL
TRISKELION_DATABASE_URL='postgres://user:pass@db-host:5432/triskelion' triskelion
# MySQL（要求 ≥ 8.0.13）/ MariaDB（≥ 10.2）
TRISKELION_DATABASE_URL='mysql://user:pass@db-host:3306/triskelion' triskelion
# 显式指定 SQLite 文件路径（等价于默认行为）
TRISKELION_DATABASE_URL='sqlite:/var/lib/triskelion/hub.db' triskelion
```

- 首次启动自动建表（幂等），无需手工执行 schema；升级版本时的补列迁移也在启动时自动完成。
- 连接池大小默认 SQLite 4 / PostgreSQL、MySQL 10，可用 `TRISKELION_DB_MAX_CONNS` 覆盖。
- 技能压缩体 blob（`data/blobs/`）与密钥（`keys/`）仍存放于文件系统数据目录，不随数据库切换。
- MySQL 建表使用 `utf8mb4_bin` 排序规则，保证用户名/资源名唯一性的大小写敏感语义与
  SQLite / PostgreSQL 一致。MariaDB 采用与 MySQL 相同的方言路径（兼容语法，未做专项实测）。

**存量数据迁移（如 SQLite → PostgreSQL）** 走管理后台的全量资源包（见下），三步完成：

```bash
# 1. 从旧实例导出
curl -H "X-Admin-Token: $ADMIN_TOKEN" http://<hub>/v1/admin/export -o backup.tskpack
# 2. 设置 TRISKELION_DATABASE_URL 指向新数据库并重启 Hub（沿用同一数据目录以保留 blob 与密钥）
# 3. 导入
curl -H "X-Admin-Token: $ADMIN_TOKEN" --data-binary @backup.tskpack http://<hub>/v1/admin/import
```

## 压缩算法（zstd）

技能压缩体与管理后台的「全量资源包」统一采用 **zstd（Zstandard）**：相比旧版 gzip，在
相近甚至更快的压缩速度下取得显著更高的压缩比，解压速度数倍于 gzip。`tsk build` 产出
`tar.zst`（级别 19、可多线程编码，兼顾压缩比与性能），按 sha256 内容寻址承载。

升级向后兼容：解压路径按文件头魔数自动识别 zstd / gzip，历史 `.tar.gz` 压缩体仍可正常
`tsk pull`、下载与迁移，无需手工转换。

## 管理后台（`ADMIN_TOKEN`）

为服务端设置 `ADMIN_TOKEN` 环境变量即启用管理后台，访问 `http://<hub>/#admin`，输入该
令牌进入（专供管理员，与普通用户登录态隔离；请求以 `X-Admin-Token` 头鉴权）：

```bash
ADMIN_TOKEN=请改成强随机串 triskelion
# 启动日志会提示：admin panel: enabled → http://127.0.0.1:8787/#admin
```

面板能力：

- **概览** —— 用户 / 技能 / MCP / 凭据规模，24h 与累计调用量、错误率，24 小时热门工具、最近错误。
- **技能 / MCP** —— 全量资源清单，可逐个**配置可见性**：private / public，以及 public 资源对**哪些分组可见**；亦可删除。
- **用户** —— 用户增删改查（CRUD）：新建账号、调整所属分组（可绑定**多个**分组）、重置密码、删除（连带其技能 / MCP / 凭据）。
- **分组** —— 分组增删改查（CRUD）。分组用于控制市场资源的可见范围；用户与分组为多对多关系，删除分组只解除成员关联，不删除用户。
- **标签** —— 受管标签增删改查（CRUD），默认内置「官方」「社区」。在技能 / MCP 的「配置」里为资源分配标签；市场可按标签筛选，资源卡片亦展示标签徽章。
- **调用日志** —— 工具调用审计（每次经 Hub 网关代调用都会记录）。
- **数据迁移** —— 一键导入 / 导出**全量资源包**（见下，已含分组、标签与可见性配置）。

### 分组与可见性

市场资源（技能 / MCP）发布为 **public** 时，**默认对所有分组可见**。管理员可在后台把某个
public 资源限定为仅指定分组可见——此时只有归属这些分组的登录用户能在市场看到、拉取、调用它
（资源作者本人始终可见）；匿名访客只能看到「所有分组可见」的资源。普通用户登录后从**个人中心**
查看与管理「我的技能 / 我的 MCP / 我的变量」；未登录则只呈现技能市场与 MCP 市场。

### 全量资源包导入 / 导出

管理后台「数据迁移」可把整个实例打包成单个 `.tskpack`（`tar` + `zstd`）下载，包含全部用户、
MCP、技能（含**版本历史**）、加密凭据、点赞/收藏、系统设置（注册开关、LDAP 配置）、调用日志，
以及按 sha256 内容寻址的全部压缩体 blob（含历史版本引用的）；在另一实例上传同一文件即可迁移。
资源包不依赖数据库类型，亦是 **SQLite ↔ PostgreSQL ↔ MySQL 之间迁移数据的官方载体**：

```bash
# 导出（也可直接调 API）
curl -H "X-Admin-Token: $ADMIN_TOKEN" http://<hub>/v1/admin/export -o backup.tskpack
# 导入（合并/upsert，按自然键覆盖同名资源，不删除已有数据）
curl -H "X-Admin-Token: $ADMIN_TOKEN" --data-binary @backup.tskpack http://<hub2>/v1/admin/import
```

> 凭据以「nonce + 密文」原样迁移，目标实例须共用同一 `master.key`（或 `TRISKELION_MASTER_KEY`）
> 方可解密。
