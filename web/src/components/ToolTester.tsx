import { useState } from "react";
import { api } from "../lib/api";
import type { JsonSchema, ToolMeta } from "../lib/types";

const inputCls =
  "w-full rounded-lg border border-slate-200 px-3 py-2 text-sm outline-none transition focus:border-indigo-300 focus:ring-4 focus:ring-indigo-500/10";

function coerce(type: string | undefined, raw: string): unknown {
  switch (type) {
    case "integer":
    case "number": {
      const n = Number(raw);
      return raw.trim() !== "" && !Number.isNaN(n) ? n : raw;
    }
    case "boolean":
      return raw === "true" || raw === "1" || raw === "yes";
    case "array":
    case "object":
      try {
        return JSON.parse(raw);
      } catch {
        return raw;
      }
    default: {
      const t = raw.trimStart();
      if (t.startsWith("[") || t.startsWith("{")) {
        try {
          return JSON.parse(raw);
        } catch {
          return raw;
        }
      }
      return raw;
    }
  }
}

function renderResult(result: unknown): { text: string; isError: boolean } {
  const r = result as { content?: { type?: string; text?: string }[]; isError?: boolean };
  if (r && Array.isArray(r.content)) {
    const text = r.content
      .map((c) => (c.type === "text" ? (c.text ?? "") : JSON.stringify(c, null, 2)))
      .join("\n");
    return { text, isError: !!r.isError };
  }
  return { text: JSON.stringify(result, null, 2), isError: false };
}

export default function ToolTester({
  owner,
  name,
  tool,
  user,
  onRequireLogin,
}: {
  owner: string;
  name: string;
  tool: ToolMeta;
  user: string | null;
  onRequireLogin: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [values, setValues] = useState<Record<string, string>>({});
  const [rawText, setRawText] = useState("{}");
  const [result, setResult] = useState<{ text: string; isError: boolean } | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  const schema: JsonSchema = tool.input_schema ?? {};
  const props = schema.properties ?? {};
  const propEntries = Object.entries(props);
  const required = schema.required ?? [];

  async function call() {
    setError("");
    setResult(null);
    let args: unknown;
    if (propEntries.length) {
      const o: Record<string, unknown> = {};
      for (const [k, sch] of propEntries) {
        const raw = values[k] ?? "";
        if (raw === "" && !required.includes(k)) continue;
        o[k] = coerce(sch.type, raw);
      }
      args = o;
    } else {
      try {
        args = rawText.trim() ? JSON.parse(rawText) : {};
      } catch {
        setError("参数 JSON 解析失败");
        return;
      }
    }
    setBusy(true);
    try {
      const res = await api.callTool(owner, name, tool.name, args);
      setResult(renderResult(res));
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="rounded-xl border border-slate-200 bg-slate-50/60 p-3.5">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <span className="font-mono text-sm font-semibold text-slate-700">{tool.name}</span>
          {tool.description && (
            <p className="mt-0.5 text-xs leading-5 text-slate-500">{tool.description}</p>
          )}
        </div>
        <button
          onClick={() => setOpen((v) => !v)}
          className="flex-none rounded-lg border border-indigo-200 px-3 py-1.5 text-xs font-semibold text-indigo-500 transition hover:bg-indigo-50"
        >
          {open ? "收起" : "测试"}
        </button>
      </div>

      {open && (
        <div className="mt-3 border-t border-slate-200 pt-3">
          {!user ? (
            <div className="flex items-center justify-between gap-3 rounded-lg bg-white px-3 py-2.5 text-sm text-slate-500">
              <span>测试调用需要登录（依赖变量时还需在「我的变量」中配置）。</span>
              <button
                onClick={onRequireLogin}
                className="flex-none rounded-lg bg-indigo-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-indigo-600"
              >
                去登录
              </button>
            </div>
          ) : (
            <>
              {propEntries.length ? (
                <div className="space-y-2.5">
                  {propEntries.map(([k, sch]) => {
                    const ty = sch.type ?? "string";
                    const struct = ty === "array" || ty === "object";
                    return (
                      <div key={k}>
                        <label className="mb-1 block text-xs font-medium text-slate-600">
                          <span className="font-mono">{k}</span>
                          <span className="ml-1.5 text-slate-400">
                            {ty}
                            {required.includes(k) ? " · 必填" : ""}
                          </span>
                          {sch.description ? (
                            <span className="ml-1.5 text-slate-400">— {sch.description}</span>
                          ) : null}
                        </label>
                        {struct ? (
                          <textarea
                            className={`${inputCls} min-h-[60px] resize-y font-mono text-xs`}
                            placeholder={ty === "array" ? "[ ... ]" : "{ ... }"}
                            value={values[k] ?? ""}
                            onChange={(e) => setValues({ ...values, [k]: e.target.value })}
                          />
                        ) : (
                          <input
                            className={inputCls}
                            value={values[k] ?? ""}
                            onChange={(e) => setValues({ ...values, [k]: e.target.value })}
                          />
                        )}
                      </div>
                    );
                  })}
                </div>
              ) : (
                <div>
                  <label className="mb-1 block text-xs font-medium text-slate-600">参数 (JSON)</label>
                  <textarea
                    className={`${inputCls} min-h-[64px] resize-y font-mono text-xs`}
                    value={rawText}
                    onChange={(e) => setRawText(e.target.value)}
                  />
                </div>
              )}

              <div className="mt-3 flex items-center gap-3">
                <button
                  onClick={call}
                  disabled={busy}
                  className="rounded-lg bg-indigo-500 px-4 py-2 text-sm font-semibold text-white transition hover:bg-indigo-600 disabled:opacity-60"
                >
                  {busy ? "调用中…" : "调用"}
                </button>
                {error && <span className="text-sm text-rose-500">{error}</span>}
              </div>

              {result && (
                <pre
                  className={`mt-3 max-h-60 overflow-auto rounded-lg border p-3 font-mono text-xs leading-relaxed ${
                    result.isError
                      ? "border-rose-200 bg-rose-50 text-rose-600"
                      : "border-slate-200 bg-white text-slate-700"
                  }`}
                >
                  {result.text || "(空结果)"}
                </pre>
              )}
            </>
          )}
        </div>
      )}
    </div>
  );
}
