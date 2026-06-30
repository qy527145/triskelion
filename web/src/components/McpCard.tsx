import type { McpInfo } from "../lib/types";
import { labelBadgeClass } from "../lib/types";
import { colorFor, initials } from "../lib/color";
import { ArrowIcon, TrashIcon } from "./icons";

export default function McpCard({
  m,
  mine,
  onDetail,
  onEdit,
  onDelete,
}: {
  m: McpInfo;
  mine?: boolean;
  onDetail: () => void;
  onEdit?: () => void;
  onDelete?: () => void;
}) {
  const color = colorFor(m.name + m.owner);
  const isPublic = m.visibility === "public";
  return (
    <div className="group flex min-h-[200px] flex-col rounded-2xl border border-slate-200/70 bg-white p-6 shadow-sm transition hover:-translate-y-0.5 hover:border-indigo-200 hover:shadow-lg hover:shadow-indigo-500/10">
      <div className="flex items-start gap-4">
        <div
          className="grid size-12 flex-none place-items-center rounded-xl text-base font-extrabold text-white"
          style={{ background: `linear-gradient(135deg, ${color}, ${color}bb)` }}
        >
          {initials(m.name)}
        </div>
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="truncate text-[17px] font-bold text-slate-800">{m.name}</h3>
            <span className="rounded-md bg-slate-100 px-2 py-0.5 font-mono text-xs font-semibold text-slate-500">
              {m.name}
            </span>
            <span className="text-xs font-medium text-slate-400">· v{m.version}</span>
          </div>
          <div className="mt-1 text-xs text-slate-400">
            @{m.owner} · {m.manifest.runtime}/{m.manifest.protocol}
          </div>
          {(m.labels ?? []).length > 0 && (
            <div className="mt-1.5 flex flex-wrap gap-1.5">
              {(m.labels ?? []).map((l) => (
                <span
                  key={l}
                  className={`rounded-md border px-1.5 py-0.5 text-xs font-medium ${labelBadgeClass(l)}`}
                >
                  {l}
                </span>
              ))}
            </div>
          )}
        </div>
      </div>

      <p className="mt-3.5 line-clamp-2 flex-1 text-sm leading-6 text-slate-500">
        {m.manifest.description || "（无描述）"}
      </p>

      <div className="mt-4 flex items-center justify-between">
        <span
          className={`rounded-lg border px-2.5 py-1 text-xs ${
            isPublic
              ? "border-emerald-200 bg-emerald-50 text-emerald-600"
              : "border-amber-200 bg-amber-50 text-amber-600"
          }`}
        >
          {m.visibility}
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
