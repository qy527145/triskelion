import { useMemo, useState } from "react";
import Modal from "./Modal";
import { api } from "../lib/api";
import type { McpInfo, McpManifest, Protocol, Runtime } from "../lib/types";

const inputCls =
  "w-full rounded-xl border border-slate-200 px-3.5 py-2.5 text-sm outline-none transition focus:border-indigo-300 focus:ring-4 focus:ring-indigo-500/10";
const labelCls = "mb-1.5 block text-xs font-semibold text-slate-600";

function parseKv(text: string): Record<string, string> {
  const o: Record<string, string> = {};
  text
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean)
    .forEach((l) => {
      const i = l.indexOf("=");
      if (i > 0) o[l.slice(0, i).trim()] = l.slice(i + 1).trim();
    });
  return o;
}

function kvToText(kv?: Record<string, string>): string {
  return Object.entries(kv ?? {})
    .map(([k, v]) => `${k}=${v}`)
    .join("\n");
}

export default function CreateMcpModal({
  edit,
  onClose,
  onSaved,
}: {
  edit?: McpInfo | null;
  onClose: () => void;
  onSaved: (name: string) => void;
}) {
  const em = edit?.manifest;
  const editing = !!edit;
  const [name, setName] = useState(em?.name ?? "");
  const [version, setVersion] = useState(em?.version ?? "0.1.0");
  const [description, setDescription] = useState(em?.description ?? "");
  const [runtime, setRuntime] = useState<Runtime>(em?.runtime ?? "remote");
  const [protocol, setProtocol] = useState<Protocol>(em?.protocol ?? "streamable");
  const [target, setTarget] = useState(
    em ? (em.runtime === "local" ? (em.command ?? "") : (em.url ?? "")) : "",
  );
  const [kvText, setKvText] = useState(
    em ? kvToText(em.runtime === "local" ? em.env : em.headers) : "",
  );
  const [visibility, setVisibility] = useState(edit?.visibility ?? "private");
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);

  const local = runtime === "local";

  function onRuntimeChange(r: Runtime) {
    setRuntime(r);
    setProtocol(r === "local" ? "stdio" : "streamable");
  }

  // 实时预览将要声明的变量
  const vars = useMemo(() => {
    const found: string[] = [];
    const scan = (s: string) => {
      const re = /\{([A-Za-z0-9_]+)\}/g;
      let mm: RegExpExecArray | null;
      while ((mm = re.exec(s))) if (!found.includes(mm[1])) found.push(mm[1]);
    };
    scan(target);
    scan(kvText);
    return found;
  }, [target, kvText]);

  async function submit() {
    setErr("");
    if (!name.trim()) return setErr("请填写名称");
    if (!target.trim()) return setErr(local ? "请填写启动命令" : "请填写 URL");
    const kv = parseKv(kvText);
    const manifest: McpManifest = {
      resource_type: "mcp",
      name: name.trim(),
      description: description.trim(),
      version: version.trim() || "0.1.0",
      runtime,
      protocol,
    };
    if (local) {
      manifest.command = target.trim();
      manifest.env = kv;
    } else {
      manifest.url = target.trim();
      manifest.headers = kv;
    }
    setBusy(true);
    try {
      // 编辑时若改了名称，先重命名再覆盖其余字段。
      const oldName = edit?.manifest.name;
      if (editing && oldName && name.trim() !== oldName) {
        await api.renameMcp(oldName, name.trim());
      }
      await api.upsertMcp(manifest, visibility);
      onSaved(name.trim());
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title={editing ? "编辑 MCP" : "新建 MCP"}
      subtitle="注册一个 MCP 工具。值中可用 {VAR} 声明运行时注入的变量。"
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
        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className={labelCls}>名称 (slug)</label>
            <input
              className={inputCls}
              value={name}
              placeholder="oa_mcp"
              onChange={(e) => setName(e.target.value)}
            />
            {editing && name.trim() !== em?.name && (
              <p className="mt-1.5 text-xs text-amber-600">
                将重命名 {em?.name} → {name.trim() || "?"}（旧的 owner/name 引用会失效）
              </p>
            )}
          </div>
          <div>
            <label className={labelCls}>版本</label>
            <input className={inputCls} value={version} onChange={(e) => setVersion(e.target.value)} />
          </div>
        </div>
        <div>
          <label className={labelCls}>描述</label>
          <input
            className={inputCls}
            value={description}
            placeholder="oa 能力封装"
            onChange={(e) => setDescription(e.target.value)}
          />
        </div>
        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className={labelCls}>运行时</label>
            <select className={inputCls} value={runtime} onChange={(e) => onRuntimeChange(e.target.value as Runtime)}>
              <option value="remote">remote</option>
              <option value="local">local</option>
            </select>
          </div>
          <div>
            <label className={labelCls}>协议</label>
            <select className={inputCls} value={protocol} onChange={(e) => setProtocol(e.target.value as Protocol)}>
              {local ? (
                <option value="stdio">stdio</option>
              ) : (
                <>
                  <option value="streamable">streamable</option>
                  <option value="sse">sse</option>
                </>
              )}
            </select>
          </div>
        </div>
        <div>
          <label className={labelCls}>{local ? "启动命令" : "URL"}</label>
          <input
            className={inputCls}
            value={target}
            placeholder={local ? "uvx acemcp --port 8888" : "http://host/mcp/{AIKO_HUB_KEY}"}
            onChange={(e) => setTarget(e.target.value)}
          />
          <p className="mt-1.5 text-xs text-slate-400">
            {local ? "本地 stdio 进程启动命令。" : "远程地址，可内嵌 {VAR} 占位符。"}
          </p>
        </div>
        <div>
          <label className={labelCls}>{local ? "环境变量（每行 KEY=VALUE，可选）" : "Headers（每行 KEY=VALUE，可选）"}</label>
          <textarea
            className={`${inputCls} min-h-[64px] resize-y font-mono text-xs`}
            value={kvText}
            placeholder={local ? "TOKEN={ACEMCP_TOKEN}" : "Authorization=Bearer {AIKO_HUB_KEY}"}
            onChange={(e) => setKvText(e.target.value)}
          />
        </div>
        {vars.length > 0 && (
          <div className="text-xs text-slate-500">
            将声明变量：
            {vars.map((v) => (
              <span
                key={v}
                className="ml-1.5 inline-block rounded-md border border-indigo-200 bg-indigo-50 px-2 py-0.5 font-mono text-indigo-500"
              >
                {v}
              </span>
            ))}
          </div>
        )}
        <div>
          <label className={labelCls}>可见性</label>
          <select className={inputCls} value={visibility} onChange={(e) => setVisibility(e.target.value)}>
            <option value="private">private（仅自己）</option>
            <option value="public">public（上架市场）</option>
          </select>
        </div>
        {err && <p className="text-sm text-rose-500">{err}</p>}
      </div>
    </Modal>
  );
}
