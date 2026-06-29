import Modal from "./Modal";
import ToolTester from "./ToolTester";
import { requiredVars, type McpInfo } from "../lib/types";

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
  const tools = m.tools ?? [];

  return (
    <Modal title={m.name} subtitle={`@${m.owner} · v${m.version} · ${m.visibility}`} onClose={onClose} wide>
      <div className="space-y-1">
        <Row label="拓扑">
          {m.manifest.runtime} / {m.manifest.protocol}
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
        </div>
        {tools.length === 0 ? (
          <p className="rounded-xl border border-dashed border-slate-200 bg-slate-50 px-4 py-3 text-xs text-slate-400">
            尚未索引到工具。owner 执行一次 <code className="font-mono">tsk run {m.owner}/{m.name}</code> 或{" "}
            <code className="font-mono">tsk mcp index {m.name}</code> 后即可在此查看与测试。
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
