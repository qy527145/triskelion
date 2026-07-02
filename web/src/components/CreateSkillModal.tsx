import { useEffect, useRef, useState } from "react";
import Modal from "./Modal";
import { api, ApiError } from "../lib/api";
import {
  docFilename,
  humanSize,
  labelBadgeClass,
  SKILL_CATEGORIES,
  type SkillCategory,
  type SkillInfo,
  type SkillManifest,
} from "../lib/types";

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
  edit,
  onClose,
  onSaved,
}: {
  edit?: SkillInfo | null;
  onClose: () => void;
  onSaved: (name: string) => void;
}) {
  const editing = !!edit;
  const [name, setName] = useState(edit?.name ?? "");
  const [version, setVersion] = useState(edit?.version ?? "0.1.0");
  const [category, setCategory] = useState<SkillCategory>(
    (edit?.category as SkillCategory) ?? "skill",
  );
  const [description, setDescription] = useState(edit?.description ?? "");
  const [tags, setTags] = useState((edit?.tags ?? []).join(", "));
  const [labels, setLabels] = useState<string[]>(edit?.labels ?? []);
  const [labelOptions, setLabelOptions] = useState<string[]>([]);
  const [mcpDeps, setMcpDeps] = useState((edit?.mcp_dependencies ?? []).join(", "));
  const [preferred, setPreferred] = useState((edit?.preferred_tools ?? []).join(", "));
  const [skillMd, setSkillMd] = useState(edit?.skill_md ?? SKILL_TEMPLATE);
  const [visibility, setVisibility] = useState(edit?.visibility ?? "private");
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);

  // 拖入压缩包创建：服务端解包归一化后，回吐的压缩体 sha256/size（提交时随元数据一并落库）。
  const [archiveSha, setArchiveSha] = useState("");
  const [archiveSize, setArchiveSize] = useState(0);
  const [archiveName, setArchiveName] = useState("");
  const [fileCount, setFileCount] = useState(0);
  const [dragActive, setDragActive] = useState(false);
  const [inspecting, setInspecting] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);

  // 受管标签清单：由后台维护，此处只能勾选已存在的（如「官方」「社区」）。
  useEffect(() => {
    api.listLabels().then(setLabelOptions).catch(() => setLabelOptions([]));
  }, []);

  // 拖入 / 选择压缩包：上传给服务端解包，用解析结果预填表单，供用户核对后创建。
  async function ingestArchive(file: File) {
    setErr("");
    setInspecting(true);
    try {
      const r = await api.inspectSkillArchive(file);
      const m = r.manifest;
      if (m.name) setName(m.name);
      if (m.version) setVersion(m.version);
      setCategory((m.category as SkillCategory) ?? "skill");
      if (m.description) setDescription(m.description);
      setTags((m.tags ?? []).join(", "));
      setMcpDeps((m.mcp_dependencies ?? []).join(", "));
      setPreferred((m.preferred_tools ?? []).join(", "));
      // 仅勾选后台已存在的受管标签，避免提交时因未知标签被拒。
      setLabels((m.labels ?? []).filter((l) => labelOptions.includes(l)));
      if (r.skill_md) setSkillMd(r.skill_md);
      setArchiveSha(r.archive_sha256);
      setArchiveSize(r.archive_size);
      setFileCount(r.file_count);
      setArchiveName(file.name);
    } catch (e) {
      setArchiveSha("");
      setArchiveName("");
      setErr(
        e instanceof ApiError
          ? `解析压缩包失败：${e.message}`
          : `解析压缩包失败：${(e as Error).message}`,
      );
    } finally {
      setInspecting(false);
    }
  }

  function onDrop(e: React.DragEvent) {
    e.preventDefault();
    setDragActive(false);
    const file = e.dataTransfer.files?.[0];
    if (file) void ingestArchive(file);
  }

  function clearArchive() {
    setArchiveSha("");
    setArchiveSize(0);
    setArchiveName("");
    setFileCount(0);
  }

  function toggleLabel(name: string) {
    setLabels((prev) => (prev.includes(name) ? prev.filter((l) => l !== name) : [...prev, name]));
  }

  // 说明书文件名随分类而定：agent → AGENT.md，其余 → SKILL.md。
  const doc = docFilename(category);

  async function submit() {
    setErr("");
    if (!name.trim()) return setErr("请填写技能名");
    if (!/^[A-Za-z0-9_.-]+$/.test(name.trim())) return setErr("技能名仅允许字母/数字/_-.");
    if (!skillMd.trim()) return setErr(`请填写 ${doc} 内容`);
    const manifest: SkillManifest = {
      name: name.trim(),
      version: version.trim() || "0.1.0",
      category,
      description: description.trim(),
      tags: parseList(tags),
      labels,
      mcp_dependencies: parseList(mcpDeps),
      preferred_tools: parseList(preferred),
    };
    setBusy(true);
    try {
      // 编辑时若改了名称，先重命名再覆盖其余字段（保留已上传的压缩体）。
      if (editing && edit && name.trim() !== edit.name) {
        await api.renameSkill(edit.owner, edit.name, name.trim());
      }
      // 拖入了压缩包则带上其 sha256/size 关联数据体；否则留空，服务端保留既有压缩体。
      await api.upsertSkill(manifest, visibility, skillMd, archiveSha, archiveSize);
      onSaved(name.trim());
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title={editing ? "编辑技能" : "新建技能"}
      subtitle={
        editing
          ? `修改技能的基础信息与 ${doc}。拖入压缩包可替换数据体，否则保持不变。`
          : "拖入压缩包（zip / tar.zst）自动解析，或直接填写创建一份裸说明书技能。"
      }
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
            {busy ? "保存中…" : editing ? "保存" : "创建"}
          </button>
        </>
      }
    >
      <div className="space-y-4">
        {/* 拖入压缩包：上传→服务端解包→预填下方表单。支持 zip / tar.zst / tar.gz / 裸 tar。 */}
        <div
          onDragOver={(e) => {
            e.preventDefault();
            setDragActive(true);
          }}
          onDragLeave={() => setDragActive(false)}
          onDrop={onDrop}
          onClick={() => fileInputRef.current?.click()}
          className={`cursor-pointer rounded-xl border-2 border-dashed px-4 py-5 text-center text-sm transition ${
            dragActive
              ? "border-indigo-400 bg-indigo-50/60"
              : archiveSha
                ? "border-emerald-300 bg-emerald-50/50"
                : "border-slate-300 bg-slate-50 hover:border-indigo-300 hover:bg-slate-100/60"
          }`}
        >
          <input
            ref={fileInputRef}
            type="file"
            accept=".zip,.tar,.zst,.tzst,.gz,.tgz"
            className="hidden"
            onChange={(e) => {
              const f = e.target.files?.[0];
              if (f) void ingestArchive(f);
              e.target.value = "";
            }}
          />
          {inspecting ? (
            <p className="text-slate-500">解析压缩包中…</p>
          ) : archiveSha ? (
            <div className="flex items-center justify-center gap-2 text-emerald-700">
              <span className="font-semibold">📦 {archiveName}</span>
              <span className="text-xs text-emerald-600">
                {fileCount} 个文件 · {humanSize(archiveSize)} · 已解析
              </span>
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation();
                  clearArchive();
                }}
                className="ml-1 rounded-lg border border-emerald-200 px-2 py-0.5 text-xs font-medium text-emerald-700 hover:bg-emerald-100"
              >
                移除
              </button>
            </div>
          ) : (
            <>
              <p className="font-medium text-slate-600">拖入压缩包，或点击选择</p>
              <p className="mt-1 text-xs text-slate-400">
                支持 .zip / .tar.zst / .tar.gz；自动读取 tsk-skill.json 与 SKILL.md 预填下方表单
              </p>
            </>
          )}
        </div>
        <div className="grid grid-cols-3 gap-3">
          <div className="col-span-2">
            <label className={labelCls}>技能名 (slug)</label>
            <input
              className={inputCls}
              value={name}
              placeholder="shield-dev-pack"
              onChange={(e) => setName(e.target.value)}
            />
            {editing && edit && name.trim() !== edit.name && (
              <p className="mt-1.5 text-xs text-amber-600">
                将重命名 {edit.name} → {name.trim() || "?"}（旧的 owner/name 引用会失效）
              </p>
            )}
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
        {labelOptions.length > 0 && (
          <div>
            <label className={labelCls}>受管标签（多选，如「官方」「社区」）</label>
            <div className="flex flex-wrap gap-2">
              {labelOptions.map((l) => {
                const on = labels.includes(l);
                return (
                  <button
                    key={l}
                    type="button"
                    onClick={() => toggleLabel(l)}
                    className={`rounded-full border px-3 py-1 text-xs font-medium transition ${
                      on
                        ? labelBadgeClass(l)
                        : "border-slate-200 bg-white text-slate-400 hover:border-slate-300"
                    }`}
                  >
                    {on ? "✓ " : ""}
                    {l}
                  </button>
                );
              })}
            </div>
          </div>
        )}
        <div>
          <label className={labelCls}>{doc}</label>
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
