import { useState } from "react";
import Modal from "./Modal";
import { api } from "../lib/api";
import { SKILL_CATEGORIES, type SkillCategory, type SkillManifest } from "../lib/types";

const inputCls =
  "w-full rounded-xl border border-slate-200 px-3.5 py-2.5 text-sm outline-none transition focus:border-indigo-300 focus:ring-4 focus:ring-indigo-500/10";
const labelCls = "mb-1.5 block text-xs font-semibold text-slate-600";

const SKILL_TEMPLATE = `# 我的技能

> 一份能力说明书。Agent 只需读这几百 Token 即可上手。

## 能力概述

描述这个技能能干什么。

## 使用方式

若依赖底层 MCP，用 tsk 包装调用：
\`\`\`bash
tsk run owner/some-mcp --help
\`\`\`
`;

function parseList(text: string): string[] {
  return text
    .split(/[,\n]/)
    .map((s) => s.trim())
    .filter(Boolean);
}

export default function CreateSkillModal({
  onClose,
  onSaved,
}: {
  onClose: () => void;
  onSaved: (name: string) => void;
}) {
  const [name, setName] = useState("");
  const [version, setVersion] = useState("0.1.0");
  const [category, setCategory] = useState<SkillCategory>("skill");
  const [description, setDescription] = useState("");
  const [tags, setTags] = useState("");
  const [mcpDeps, setMcpDeps] = useState("");
  const [preferred, setPreferred] = useState("");
  const [skillMd, setSkillMd] = useState(SKILL_TEMPLATE);
  const [visibility, setVisibility] = useState("private");
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);

  async function submit() {
    setErr("");
    if (!name.trim()) return setErr("请填写技能名");
    if (!/^[A-Za-z0-9_.-]+$/.test(name.trim())) return setErr("技能名仅允许字母/数字/_-.");
    if (!skillMd.trim()) return setErr("请填写 SKILL.md 内容");
    const manifest: SkillManifest = {
      name: name.trim(),
      version: version.trim() || "0.1.0",
      category,
      description: description.trim(),
      tags: parseList(tags),
      mcp_dependencies: parseList(mcpDeps),
      preferred_tools: parseList(preferred),
    };
    setBusy(true);
    try {
      await api.upsertSkill(manifest, visibility, skillMd);
      onSaved(name.trim());
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title="新建技能（纯文本）"
      subtitle="在 Web 端直接创建一份裸说明书技能。需要打包大文件夹时请用 tsk skill publish。"
      onClose={onClose}
      wide
      footer={
        <>
          <button
            onClick={onClose}
            className="rounded-xl border border-slate-200 px-4 py-2.5 text-sm font-semibold text-slate-600 hover:bg-slate-50"
          >
            取消
          </button>
          <button
            onClick={submit}
            disabled={busy}
            className="rounded-xl bg-indigo-500 px-5 py-2.5 text-sm font-semibold text-white hover:bg-indigo-600 disabled:opacity-60"
          >
            {busy ? "保存中…" : "创建"}
          </button>
        </>
      }
    >
      <div className="space-y-4">
        <div className="grid grid-cols-3 gap-3">
          <div className="col-span-2">
            <label className={labelCls}>技能名 (slug)</label>
            <input
              className={inputCls}
              value={name}
              placeholder="shield-dev-pack"
              onChange={(e) => setName(e.target.value)}
            />
          </div>
          <div>
            <label className={labelCls}>版本</label>
            <input className={inputCls} value={version} onChange={(e) => setVersion(e.target.value)} />
          </div>
        </div>
        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className={labelCls}>分类</label>
            <select
              className={inputCls}
              value={category}
              onChange={(e) => setCategory(e.target.value as SkillCategory)}
            >
              {SKILL_CATEGORIES.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.label}（{c.id}）
                </option>
              ))}
            </select>
          </div>
          <div>
            <label className={labelCls}>可见性</label>
            <select className={inputCls} value={visibility} onChange={(e) => setVisibility(e.target.value)}>
              <option value="private">private（仅自己）</option>
              <option value="public">public（上架市场）</option>
            </select>
          </div>
        </div>
        <div>
          <label className={labelCls}>描述</label>
          <input
            className={inputCls}
            value={description}
            placeholder="一句话说明能力"
            onChange={(e) => setDescription(e.target.value)}
          />
        </div>
        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className={labelCls}>标签（逗号/换行分隔）</label>
            <input
              className={inputCls}
              value={tags}
              placeholder="github, ci"
              onChange={(e) => setTags(e.target.value)}
            />
          </div>
          <div>
            <label className={labelCls}>依赖 MCP（owner/name）</label>
            <input
              className={inputCls}
              value={mcpDeps}
              placeholder="alice/github-inspector"
              onChange={(e) => setMcpDeps(e.target.value)}
            />
          </div>
        </div>
        <div>
          <label className={labelCls}>倾向工具（可选）</label>
          <input
            className={inputCls}
            value={preferred}
            placeholder="github-inspector/create_issue"
            onChange={(e) => setPreferred(e.target.value)}
          />
        </div>
        <div>
          <label className={labelCls}>SKILL.md</label>
          <textarea
            className={`${inputCls} min-h-[180px] resize-y font-mono text-xs leading-relaxed`}
            value={skillMd}
            onChange={(e) => setSkillMd(e.target.value)}
          />
        </div>
        {err && <p className="text-sm text-rose-500">{err}</p>}
      </div>
    </Modal>
  );
}
