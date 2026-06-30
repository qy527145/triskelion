import type { SkillInfo } from "../lib/types";
import { categoryLabel, humanSize, labelBadgeClass } from "../lib/types";
import { colorFor, initials } from "../lib/color";
import { ArrowIcon, BookIcon, SparkIcon, TrashIcon, WrenchIcon } from "./icons";

function CategoryIcon({ category, ...p }: { category: string } & React.SVGProps<SVGSVGElement>) {
  if (category === "kb") return <BookIcon {...p} />;
  if (category === "toolchain") return <WrenchIcon {...p} />;
  return <SparkIcon {...p} />;
}

const catBadge: Record<string, string> = {
  skill: "border-indigo-200 bg-indigo-50 text-indigo-600",
  kb: "border-sky-200 bg-sky-50 text-sky-600",
  toolchain: "border-amber-200 bg-amber-50 text-amber-600",
};

export default function SkillCard({
  s,
  mine,
  onDetail,
  onEdit,
  onDelete,
}: {
  s: SkillInfo;
  mine?: boolean;
  onDetail: () => void;
  onEdit?: () => void;
  onDelete?: () => void;
}) {
  const color = colorFor(s.name + s.owner);
  const isPublic = s.visibility === "public";
  return (
    <div className="group flex min-h-[210px] flex-col rounded-2xl border border-slate-200/70 bg-white p-6 shadow-sm transition hover:-translate-y-0.5 hover:shadow-lg">
      <div className="flex items-start gap-4">
        <div
          className="grid size-12 flex-none place-items-center rounded-xl text-base font-extrabold text-white"
          style={{ background: `linear-gradient(135deg, ${color}, ${color}bb)` }}
        >
          {initials(s.name)}
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="truncate text-[17px] font-bold text-slate-800">{s.name}</h3>
            <span className="text-xs font-medium text-slate-400">· v{s.version}</span>
          </div>
          <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-slate-400">
            <span
              className={`inline-flex items-center gap-1 rounded-md border px-1.5 py-0.5 font-medium ${
                catBadge[s.category] ?? catBadge.skill
              }`}
            >
              <CategoryIcon category={s.category} width={12} height={12} />
              {categoryLabel(s.category)}
            </span>
            {(s.labels ?? []).map((l) => (
              <span
                key={l}
                className={`rounded-md border px-1.5 py-0.5 font-medium ${labelBadgeClass(l)}`}
              >
                {l}
              </span>
            ))}
            <span>@{s.owner}</span>
          </div>
        </div>
      </div>

      <p className="mt-3.5 line-clamp-2 flex-1 text-sm leading-6 text-slate-500">
        {s.description || "（无描述）"}
      </p>

      {(s.tags.length > 0 || s.mcp_dependencies.length > 0) && (
        <div className="mt-3 flex flex-wrap gap-1.5">
          {s.tags.slice(0, 4).map((t) => (
            <span key={t} className="rounded-md bg-slate-100 px-2 py-0.5 text-xs text-slate-500">
              #{t}
            </span>
          ))}
          {s.mcp_dependencies.length > 0 && (
            <span className="rounded-md border border-violet-200 bg-violet-50 px-2 py-0.5 text-xs text-violet-600">
              MCP×{s.mcp_dependencies.length}
            </span>
          )}
        </div>
      )}

      <div className="mt-4 flex items-center justify-between">
        <span className="text-xs text-slate-400">
          {s.archive_size > 0 ? `📦 ${humanSize(s.archive_size)}` : "纯文本"}
          {mine && (
            <span
              className={`ml-2 rounded-md border px-1.5 py-0.5 ${
                isPublic
                  ? "border-emerald-200 bg-emerald-50 text-emerald-600"
                  : "border-amber-200 bg-amber-50 text-amber-600"
              }`}
            >
              {s.visibility}
            </span>
          )}
        </span>
        {mine ? (
          <div className="flex gap-2">
            <button
              onClick={onDetail}
              className="rounded-lg border border-slate-200 px-3 py-1.5 text-xs font-semibold text-slate-500 transition hover:bg-slate-50"
            >
              详情
            </button>
            <button
              onClick={onEdit}
              className="rounded-lg border border-indigo-200 px-3 py-1.5 text-xs font-semibold text-indigo-500 transition hover:bg-indigo-50"
            >
              编辑
            </button>
            <button
              onClick={onDelete}
              className="flex items-center gap-1 rounded-lg border border-rose-200 px-2.5 py-1.5 text-xs font-semibold text-rose-500 transition hover:bg-rose-50"
            >
              <TrashIcon width={13} height={13} />
            </button>
          </div>
        ) : (
          <button
            onClick={onDetail}
            className="flex items-center gap-1 text-sm font-semibold text-indigo-500 transition group-hover:gap-1.5"
          >
            查看详情 <ArrowIcon width={15} height={15} />
          </button>
        )}
      </div>
    </div>
  );
}
