import { useEffect, useRef, useState } from "react";
import Modal from "./Modal";
import ToolTester from "./ToolTester";
import ReactionBar from "./ReactionBar";
import { api } from "../lib/api";
import { requiredVars, type McpInfo, type ToolMeta } from "../lib/types";

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex gap-3 py-1 text-sm">
      <span className="w-16 flex-none font-semibold text-slate-500">{label}</span>
      <span className="min-w-0 break-words text-slate-700">{children}</span>
    </div>
  );
}

export default function DetailModal({
  m,
  user,
  onClose,
  onRequireLogin,
}: {
  m: McpInfo;
  user: string | null;
  onClose: () => void;
  onRequireLogin: () => void;
}) {
  const vars = requiredVars(m.manifest);
  const target = m.manifest.runtime === "local" ? m.manifest.command : m.manifest.url;
  const runCmd = `tsk run ${m.owner}/${m.name} --help`;

  const [tools, setTools] = useState<ToolMeta[]>(m.tools ?? []);
  const [indexing, setIndexing] = useState(false);
  const [indexError, setIndexError] = useState("");
  const autoIndexed = useRef(false);

  async function reindex(silent = false) {
    setIndexing(true);
    if (!silent) setIndexError("");
    try {
      const res = await api.indexMcpTools(m.owner, m.name);
      setTools(res.tools);
      setIndexError("");
    } catch (e) {
      // 静默模式（打开详情时的后台刷新且已有缓存）失败不打扰：
      // 查看者可能缺变量或 MCP 暂不可达，保留落库清单即可。
      if (!silent) setIndexError((e as Error).message);
    } finally {
      setIndexing(false);
    }
  }

  // 打开详情即后台重新索引（stale-while-revalidate）：先展示落库的旧清单，
  // 拉到实时清单后静默替换并落库，保证查看与测试面对的是 MCP 当前的工具集。
  useEffect(() => {
    if (autoIndexed.current || !user) return;
    autoIndexed.current = true;
    void reindex((m.tools ?? []).length > 0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <Modal title={m.name} subtitle={`@${m.owner} · v${m.version} · ${m.visibility}`} onClose={onClose} wide>
      <div className="space-y-1">
        <Row label="拓扑">
          {m.manifest.runtime} / {m.manifest.protocol}
        </Row>
        <Row label="互动">
          <ReactionBar likes={m.likes} favorites={m.favorites} liked={m.liked} favorited={m.favorited} />
        </Row>
        <Row label={m.manifest.runtime === "local" ? "命令" : "地址"}>{target || "—"}</Row>
        <Row label="所需变量">
          {vars.length ? (
            vars.map((v) => (
              <span
                key={v}
                className="mr-1.5 inline-block rounded-md border border-indigo-200 bg-indigo-50 px-2 py-0.5 font-mono text-xs text-indigo-500"
              >
                {v}
              </span>
            ))
          ) : (
            <span className="text-slate-400">无</span>
          )}
        </Row>
        <Row label="运行">
          <code className="rounded-md bg-slate-100 px-2 py-0.5 font-mono text-xs">{runCmd}</code>
        </Row>
      </div>

      <div className="mt-5">
        <div className="mb-2 flex items-center gap-2 text-sm font-semibold text-slate-700">
          工具
          <span className="rounded-full bg-slate-100 px-2 py-0.5 text-xs font-medium text-slate-500">
            {tools.length}
          </span>
          {user && (
            <button
              onClick={() => void reindex()}
              disabled={indexing}
              className="rounded-lg border border-slate-200 px-2.5 py-1 text-xs font-medium text-slate-500 transition hover:bg-slate-50 disabled:opacity-60"
            >
              {indexing ? "索引中…" : "重新索引"}
            </button>
          )}
          {indexError && tools.length > 0 && (
            <span className="min-w-0 truncate text-xs font-normal text-rose-500">{indexError}</span>
          )}
        </div>
        {tools.length === 0 ? (
          <p className="rounded-xl border border-dashed border-slate-200 bg-slate-50 px-4 py-3 text-xs text-slate-400">
            {indexing ? (
              "正在连接 MCP 索引工具…"
            ) : !user ? (
              "登录后打开详情将自动索引工具，即可在此查看与测试。"
            ) : (
              <>
                {indexError ? `自动索引失败：${indexError}` : "尚未索引到工具。"}
                {" 可点击上方「重新索引」重试，或由 owner 执行 "}
                <code className="font-mono">tsk mcp index {m.name}</code>。
              </>
            )}
          </p>
        ) : (
          <div className="space-y-2.5">
            {tools.map((t) => (
              <ToolTester
                key={t.name}
                owner={m.owner}
                name={m.name}
                tool={t}
                user={user}
                onRequireLogin={onRequireLogin}
              />
            ))}
          </div>
        )}
      </div>

      <div className="mt-5">
        <div className="mb-1.5 text-xs font-semibold text-slate-600">manifest</div>
        <pre className="max-h-60 overflow-auto rounded-xl border border-slate-200 bg-slate-50 p-4 font-mono text-xs leading-relaxed text-slate-700">
          {JSON.stringify(m.manifest, null, 2)}
        </pre>
      </div>
    </Modal>
  );
}
