import { useEffect, useState } from "react";
import Modal from "./Modal";
import Markdown from "./Markdown";
import ReactionBar from "./ReactionBar";
import { api } from "../lib/api";
import {
  categoryLabel,
  docFilename,
  humanSize,
  type SkillInfo,
  type SkillVersionInfo,
} from "../lib/types";
import { DownloadIcon } from "./icons";

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex gap-3 py-1 text-sm">
      <span className="w-20 flex-none font-semibold text-slate-500">{label}</span>
      <span className="min-w-0 flex-1 break-words text-slate-700">{children}</span>
    </div>
  );
}

/** 去掉时间串的 " UTC" 尾巴，紧凑展示。 */
function shortTime(t: string): string {
  return t.replace(" UTC", "");
}

export default function SkillDetailModal({ s, onClose }: { s: SkillInfo; onClose: () => void }) {
  // 版本历史（新→旧）；view 为当前查看的版本内容（默认最新版，即入参 s）。
  const [versions, setVersions] = useState<SkillVersionInfo[] | null>(null);
  const [view, setView] = useState<SkillInfo>(s);
  const [loadingVer, setLoadingVer] = useState(false);
  const isHead = view.version === s.version;

  useEffect(() => {
    let alive = true;
    api
      .skillVersions(s.owner, s.name)
      .then((list) => alive && setVersions(list))
      .catch(() => alive && setVersions([]));
    return () => {
      alive = false;
    };
  }, [s.owner, s.name]);

  const switchVersion = (ver: string) => {
    if (ver === view.version) return;
    if (ver === s.version) {
      setView(s); // 回到最新版：直接用列表已有数据，无需重新请求
      return;
    }
    setLoadingVer(true);
    api
      .getSkill(s.owner, s.name, ver)
      .then(setView)
      .catch(() => setView(s))
      .finally(() => setLoadingVer(false));
  };

  const pullCmd = isHead
    ? `tsk pull ${s.owner}/${s.name}`
    : `tsk pull ${s.owner}/${s.name}@${view.version}`;
  const doc = docFilename(view.category);
  const viewVer = versions?.find((v) => v.version === view.version);

  return (
    <Modal
      title={s.name}
      subtitle={`@${s.owner} · v${view.version} · ${categoryLabel(s.category)} · ${s.visibility}`}
      onClose={onClose}
      wide
    >
      <div className="space-y-1">
        <Row label="分类">{categoryLabel(s.category)}</Row>
        <Row label="互动">
          <ReactionBar
            likes={s.likes}
            favorites={s.favorites}
            downloads={s.downloads}
            liked={s.liked}
            favorited={s.favorited}
          />
        </Row>
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
        <Row label="版本">
          {versions === null ? (
            <span className="text-slate-400">加载中…</span>
          ) : (
            <div className="space-y-1.5">
              <div className="flex flex-wrap items-center gap-1.5">
                {(versions.length ? versions : [{ version: s.version } as SkillVersionInfo]).map(
                  (v) => {
                    const active = v.version === view.version;
                    const head = v.version === s.version;
                    return (
                      <button
                        key={v.version}
                        onClick={() => switchVersion(v.version)}
                        title={
                          (v.created_at ? `发布于 ${shortTime(v.created_at)}` : "") +
                          (v.archive_size ? ` · ${humanSize(v.archive_size)}` : "")
                        }
                        className={
                          "rounded-md border px-2 py-0.5 font-mono text-xs transition " +
                          (active
                            ? "border-indigo-400 bg-indigo-500 font-semibold text-white"
                            : "border-slate-200 bg-white text-slate-600 hover:border-indigo-300 hover:text-indigo-600")
                        }
                      >
                        v{v.version}
                        {head && (
                          <span className={active ? "ml-1 opacity-80" : "ml-1 text-emerald-500"}>
                            •最新
                          </span>
                        )}
                      </button>
                    );
                  },
                )}
              </div>
              {viewVer && (
                <div className="text-xs text-slate-400">
                  该版本发布于 {shortTime(viewVer.created_at)}
                  {viewVer.archive_size > 0 && <> · 压缩体 {humanSize(viewVer.archive_size)}</>}
                  ；重复发布同一版本号会覆盖该版本
                </div>
              )}
            </div>
          )}
        </Row>
        <Row label="依赖 MCP">
          {view.mcp_dependencies.length ? (
            <div className="space-y-1">
              {view.mcp_dependencies.map((d) => (
                <code key={d} className="block rounded-md bg-slate-100 px-2 py-0.5 font-mono text-xs">
                  tsk run {d} --help
                </code>
              ))}
            </div>
          ) : (
            <span className="text-slate-400">无（纯文本裸说明书）</span>
          )}
        </Row>
        {view.preferred_tools.length > 0 && (
          <Row label="倾向工具">
            {view.preferred_tools.map((t) => (
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

      {!isHead && (
        <p className="mt-3 rounded-lg border border-amber-200 bg-amber-50 px-3 py-1.5 text-xs text-amber-700">
          正在查看历史版本 v{view.version}（最新版 v{s.version}）。
          <button className="ml-1 font-semibold underline" onClick={() => switchVersion(s.version)}>
            回到最新版
          </button>
        </p>
      )}

      {view.archive_size > 0 && (
        <a
          href={api.skillArchiveUrl(s.owner, s.name, isHead ? undefined : view.version)}
          className="mt-4 inline-flex items-center gap-2 rounded-xl border border-indigo-200 bg-indigo-50 px-4 py-2 text-sm font-semibold text-indigo-600 transition hover:bg-indigo-100"
        >
          <DownloadIcon width={16} height={16} /> 下载技能包 v{view.version} (
          {humanSize(view.archive_size)})
        </a>
      )}

      <div className="mt-5">
        <div className="mb-2 text-sm font-semibold text-slate-700">
          {doc}
          {!isHead && <span className="ml-2 font-normal text-slate-400">v{view.version}</span>}
        </div>
        {loadingVer ? (
          <p className="rounded-xl border border-dashed border-slate-200 bg-slate-50 px-4 py-3 text-xs text-slate-400">
            加载 v{view.version} 中…
          </p>
        ) : view.skill_md.trim() ? (
          <div className="max-h-[46vh] overflow-auto rounded-xl border border-slate-200 bg-white px-5 py-3">
            <Markdown text={view.skill_md} />
          </div>
        ) : (
          <p className="rounded-xl border border-dashed border-slate-200 bg-slate-50 px-4 py-3 text-xs text-slate-400">
            该技能未提供 {doc} 文本。
          </p>
        )}
      </div>
    </Modal>
  );
}
