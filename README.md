# triskelion

Welcome to the Triskelion. Equip your AI Agents with tactical Skills, localized Knowledge Intel, and bulletproof CLI tools.

万物皆 **Skill**：技能（skill）、知识库（kb）、工具链（toolchain）只是逻辑分类标签，底层共用同一数据结构。Agent 初始化时只读几百 Token 的精简 `SKILL.md`，需要动作时才用 `tsk` 触发底层 MCP。

## 两个市场

- **技能市场（Skill）** —— 平台首页。技能包是一个文件夹（必须含 `SKILL.md`），发布前由 `tsk build` 打成 `tar.zst`。服务端只持元数据 + `SKILL.md` 文本，庞大的数据体以压缩包形式按 sha256 内容寻址承载。支持按分类、标签、关键字检索。
- **MCP 市场** —— 注册可被 `tsk run` 当 CLI 调用的 MCP 工具，声明运行拓扑（local / remote）与所需 `{VAR}` 变量。

## 技能全生命周期（tsk）

```bash
# 1) 脚手架：生成 SKILL.md + tsk-skill.json
tsk skill init my-skill && cd my-skill

# 2) 本地打包校验（产出 .tsk/dist/<name>-<version>.tar.gz）
tsk build                       # 等价于 tsk skill build

# 3) 发布（build + 上传元数据/SKILL.md + 上传压缩体）
tsk skill publish --visibility public

# 4) 发现与拉取
tsk skill search "" --category toolchain --tag github
tsk pull alice/shield-dev-pack        # 下载、校验 sha256、解压到 ./shield-dev-pack
tsk skill show alice/shield-dev-pack  # 查看元数据 + SKILL.md
tsk skill list                        # 名下全部技能（含私有）
```

### `tsk-skill.json`

```jsonc
{
  "name": "shield-dev-pack",
  "version": "1.0.0",
  "category": "toolchain",            // skill | kb | toolchain
  "description": "盾局开发工具链",
  "tags": ["github", "ci"],
  "mcp_dependencies": ["alice/github-inspector"],   // 依赖的底层 MCP
  "preferred_tools": ["github-inspector/create_issue"]
}
```

技能若依赖底层 MCP，在 `SKILL.md` 里用 `tsk` 包装调用即可（说明书会随 `tsk pull` 一并提示）：

```bash
tsk run alice/github-inspector --help
tsk run alice/github-inspector create_issue --title "..." --body "..."
```

纯文本「裸说明书」技能无需任何压缩体，也可在 Web 端「我的技能 → 新建技能」直接创建。

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
- **技能 / MCP / 用户 / 调用日志** —— 全量资源清单与工具调用审计（每次经 Hub 网关代调用都会记录）。
- **数据迁移** —— 一键导入 / 导出**全量资源包**（见下）。

### 全量资源包导入 / 导出

管理后台「数据迁移」可把整个实例打包成单个 `.tskpack`（`tar` + `zstd`）下载，包含全部用户、
MCP、技能、加密凭据、调用日志，以及按 sha256 内容寻址的全部压缩体 blob；在另一实例上传同一
文件即可迁移：

```bash
# 导出（也可直接调 API）
curl -H "X-Admin-Token: $ADMIN_TOKEN" http://<hub>/v1/admin/export -o backup.tskpack
# 导入（合并/upsert，按自然键覆盖同名资源，不删除已有数据）
curl -H "X-Admin-Token: $ADMIN_TOKEN" --data-binary @backup.tskpack http://<hub2>/v1/admin/import
```

> 凭据以「nonce + 密文」原样迁移，目标实例须共用同一 `master.key`（或 `TRISKELION_MASTER_KEY`）
> 方可解密。
