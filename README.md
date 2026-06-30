# triskelion

Welcome to the Triskelion. Equip your AI Agents with tactical Skills, localized Knowledge Intel, and bulletproof CLI tools.

万物皆 **Skill**：技能（skill）、知识库（kb）、工具链（toolchain）只是逻辑分类标签，底层共用同一数据结构。Agent 初始化时只读几百 Token 的精简 `SKILL.md`，需要动作时才用 `tsk` 触发底层 MCP。

## 两个市场

- **技能市场（Skill）** —— 平台首页。技能包是一个文件夹（必须含 `SKILL.md`），发布前由 `tsk build` 打成 `tar.gz`。服务端只持元数据 + `SKILL.md` 文本，庞大的数据体以压缩包形式按 sha256 内容寻址承载。支持按分类、标签、关键字检索。
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
