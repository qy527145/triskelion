import Modal from "./Modal";
import Markdown from "./Markdown";
import { api } from "../lib/api";
import { categoryLabel, humanSize, type SkillInfo } from "../lib/types";
import { DownloadIcon } from "./icons";

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex gap-3 py-1 text-sm">
      <span className="w-20 flex-none font-semibold text-slate-500">{label}</span>
      <span className="min-w-0 break-words text-slate-700">{children}</span>
    </div>
  );
}

export default function SkillDetailModal({ s, onClose }: { s: SkillInfo; onClose: () => void }) {
  const pullCmd = `tsk pull ${s.owner}/${s.name}`;
  return (
    <Modal
      title={s.name}
      subtitle={`@${s.owner} · v${s.version} · ${categoryLabel(s.category)} · ${s.visibility}`}
      onClose={onClose}
      wide
    >
      <div className="space-y-1">
        <Row label="分类">{categoryLabel(s.category)}</Row>
        <Row label="描述">{s.description || <span className="text-slate-400">—</span>}</Row>
        <Row label="标签">
          {s.tags.length ? (
            s.tags.map((t) => (
              <span
                key={t}
                className="mr-1.5 inline-block rounded-md bg-slate-100 px-2 py-0.5 text-xs text-slate-500"
              >
                #{t}
              </span>
            ))
          ) : (
            <span className="text-slate-400">无</span>
          )}
        </Row>
        <Row label="依赖 MCP">
          {s.mcp_dependencies.length ? (
            <div className="space-y-1">
              {s.mcp_dependencies.map((d) => (
                <code key={d} className="block rounded-md bg-slate-100 px-2 py-0.5 font-mono text-xs">
                  tsk run {d} --help
                </code>
              ))}
            </div>
          ) : (
            <span className="text-slate-400">无（纯文本裸说明书）</span>
          )}
        </Row>
        {s.preferred_tools.length > 0 && (
          <Row label="倾向工具">
            {s.preferred_tools.map((t) => (
              <span
                key={t}
                className="mr-1.5 inline-block rounded-md border border-violet-200 bg-violet-50 px-2 py-0.5 font-mono text-xs text-violet-600"
              >
                {t}
              </span>
            ))}
          </Row>
        )}
        <Row label="拉取">
          <code className="rounded-md bg-slate-100 px-2 py-0.5 font-mono text-xs">{pullCmd}</code>
        </Row>
      </div>

      {s.archive_size > 0 && (
        <a
          href={api.skillArchiveUrl(s.owner, s.name)}
          className="mt-4 inline-flex items-center gap-2 rounded-xl border border-indigo-200 bg-indigo-50 px-4 py-2 text-sm font-semibold text-indigo-600 transition hover:bg-indigo-100"
        >
          <DownloadIcon width={16} height={16} /> 下载技能包 ({humanSize(s.archive_size)})
        </a>
      )}

      <div className="mt-5">
        <div className="mb-2 text-sm font-semibold text-slate-700">SKILL.md</div>
        {s.skill_md.trim() ? (
          <div className="max-h-[46vh] overflow-auto rounded-xl border border-slate-200 bg-white px-5 py-3">
            <Markdown text={s.skill_md} />
          </div>
        ) : (
          <p className="rounded-xl border border-dashed border-slate-200 bg-slate-50 px-4 py-3 text-xs text-slate-400">
            该技能未提供 SKILL.md 文本。
          </p>
        )}
      </div>
    </Modal>
  );
}
