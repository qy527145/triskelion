import { useCallback, useEffect, useRef, useState } from "react";
import { admin, ApiError } from "../lib/api";
import { docFilename, humanSize, parseGroupVisibility, SKILL_CATEGORIES } from "../lib/types";
import type {
  AdminGroup,
  AdminLabel,
  AdminMcp,
  AdminSkill,
  AdminStats,
  AdminUser,
  CallLog,
  CallsQuery,
  CallsResp,
  GroupVisibility,
  ImportSummary,
  McpManifest,
  Protocol,
  Runtime,
  SkillCategory,
  SkillVersionInfo,
} from "../lib/types";
import Modal from "./Modal";
import Brand from "./Brand";
import { DownloadIcon, LogoutIcon, PlusIcon, Spinner, TrashIcon } from "./icons";

const TOKEN_KEY = "tsk_admin_token";

/** 侧边导航按「资源 / 成员 / 运维」分组，把相关页面组织在一起。 */
const NAV_SECTIONS = [
  {
    section: "资源管理",
    tabs: [
      { id: "resources", label: "资源" },
      { id: "labels", label: "标签" },
    ],
  },
  {
    section: "成员管理",
    tabs: [
      { id: "users", label: "用户" },
      { id: "groups", label: "分组" },
    ],
  },
  {
    section: "运维",
    tabs: [
      { id: "overview", label: "概览" },
      { id: "calls", label: "调用日志" },
      { id: "migrate", label: "数据迁移" },
    ],
  },
] as const;
type AdminTab = (typeof NAV_SECTIONS)[number]["tabs"][number]["id"];

export default function AdminPanel() {
  const [token, setToken] = useState<string | null>(() => sessionStorage.getItem(TOKEN_KEY));
  const [authed, setAuthed] = useState(false);

  if (!token || !authed) {
    return (
      <TokenGate
        onAuthed={(t) => {
          sessionStorage.setItem(TOKEN_KEY, t);
          setToken(t);
          setAuthed(true);
        }}
      />
    );
  }
  return (
    <Dashboard
      token={token}
      onLogout={() => {
        sessionStorage.removeItem(TOKEN_KEY);
        setToken(null);
        setAuthed(false);
      }}
    />
  );
}

// ---------------------------------------------------------------------------
// 令牌入口
// ---------------------------------------------------------------------------

function TokenGate({ onAuthed }: { onAuthed: (token: string) => void }) {
  const [value, setValue] = useState(sessionStorage.getItem(TOKEN_KEY) ?? "");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  async function submit() {
    const t = value.trim();
    if (!t) return;
    setBusy(true);
    setError("");
    try {
      await admin.stats(t); // 验证令牌
      onAuthed(t);
    } catch (e) {
      const err = e as ApiError;
      setError(
        err.status === 503
          ? "管理后台未启用：服务端未设置 ADMIN_TOKEN 环境变量。"
          : err.message || "令牌校验失败",
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="grid min-h-screen place-items-center px-6">
      <div className="w-full max-w-sm rounded-2xl border border-slate-200/70 bg-white/90 p-8 shadow-xl shadow-slate-300/30 backdrop-blur">
        <div className="mb-6">
          <Brand sub="管理后台" />
        </div>
        <p className="mb-5 text-sm text-slate-400">
          输入 <code className="rounded bg-slate-100 px-1 py-0.5 font-mono text-xs">ADMIN_TOKEN</code>{" "}
          进入。仅平台管理员可用。
        </p>
        <input
          type="password"
          autoFocus
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && submit()}
          placeholder="ADMIN_TOKEN"
          className="w-full rounded-xl border border-slate-200 bg-white px-4 py-2.5 text-sm outline-none transition focus:border-indigo-300 focus:ring-4 focus:ring-indigo-500/10"
        />
        {error && <div className="mt-3 text-sm text-rose-500">{error}</div>}
        <button
          onClick={submit}
          disabled={busy}
          className="mt-5 w-full rounded-xl bg-indigo-500 px-4 py-2.5 text-sm font-semibold text-white transition hover:bg-indigo-600 disabled:opacity-60"
        >
          {busy ? "校验中…" : "进入"}
        </button>
        <a href="#" className="mt-4 block text-center text-xs text-slate-400 hover:text-indigo-500">
          ← 返回市场
        </a>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// 仪表盘
// ---------------------------------------------------------------------------

function Dashboard({ token, onLogout }: { token: string; onLogout: () => void }) {
  const [tab, setTab] = useState<AdminTab>("resources");
  const [toast, setToast] = useState("");
  const notify = useCallback((m: string) => {
    setToast(m);
    setTimeout(() => setToast(""), 2400);
  }, []);

  return (
    <div className="min-h-screen">
      <header className="sticky top-0 z-40 flex items-center gap-3 border-b border-slate-200/70 bg-white/80 px-6 py-3.5 backdrop-blur-xl">
        <Brand sub="· 管理后台" />
        <div className="flex-1" />
        <a href="#" className="rounded-lg px-3 py-1.5 text-sm text-slate-500 transition hover:bg-slate-100">
          市场首页
        </a>
        <button
          onClick={onLogout}
          className="flex items-center gap-1 rounded-lg px-2 py-1.5 text-sm text-slate-500 transition hover:bg-slate-100 hover:text-rose-500"
        >
          <LogoutIcon width={16} height={16} /> 退出
        </button>
      </header>

      <main className="mx-auto max-w-[1400px] px-6 py-9">
        <h1 className="text-[28px] font-bold tracking-wide text-slate-800">管理后台</h1>
        <p className="mt-2 text-slate-400">技能、MCP 服务、用户、工具调用审计与全量资源包迁移。</p>

        <div className="mt-7 flex gap-6">
          <nav className="flex w-44 flex-none flex-col gap-4 self-start rounded-2xl border border-slate-200 bg-white p-2 shadow-sm">
            {NAV_SECTIONS.map((sec) => (
              <div key={sec.section} className="flex flex-col gap-1">
                <div className="px-3 pt-1 text-[11px] font-semibold uppercase tracking-wider text-slate-300">
                  {sec.section}
                </div>
                {sec.tabs.map((t) => (
                  <button
                    key={t.id}
                    onClick={() => setTab(t.id)}
                    className={`rounded-xl px-3.5 py-2.5 text-left text-sm font-medium transition ${
                      tab === t.id
                        ? "bg-indigo-50 font-semibold text-indigo-600"
                        : "text-slate-600 hover:bg-slate-100"
                    }`}
                  >
                    {t.label}
                  </button>
                ))}
              </div>
            ))}
          </nav>

          <div className="min-w-0 flex-1">
            {tab === "overview" && <Overview token={token} />}
            {tab === "resources" && <ResourcesManager token={token} notify={notify} />}
            {tab === "users" && <UsersTable token={token} notify={notify} />}
            {tab === "groups" && <GroupsTable token={token} notify={notify} />}
            {tab === "labels" && <LabelsTable token={token} notify={notify} />}
            {tab === "calls" && <CallsTable token={token} />}
            {tab === "migrate" && <Migrate token={token} notify={notify} />}
          </div>
        </div>
      </main>

      {toast && (
        <div className="fixed bottom-7 left-1/2 z-[100] -translate-x-1/2 rounded-xl bg-slate-800 px-4 py-2.5 text-sm text-white shadow-2xl">
          {toast}
        </div>
      )}
    </div>
  );
}

/** 通用数据拉取 hook：把 ApiError 文案抛给 UI。 */
function useAdminData<T>(fetcher: () => Promise<T>, deps: unknown[]) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const fn = useCallback(fetcher, deps);
  const reload = useCallback(() => {
    setLoading(true);
    setError("");
    fn()
      .then(setData)
      .catch((e) => setError((e as Error).message))
      .finally(() => setLoading(false));
  }, [fn]);
  useEffect(() => {
    reload();
  }, [reload]);
  return { data, error, loading, reload };
}

function Panel({ children, className = "" }: { children: React.ReactNode; className?: string }) {
  return (
    <div className={`rounded-2xl border border-slate-200 bg-white p-6 shadow-sm ${className}`}>
      {children}
    </div>
  );
}

function StateLine({ loading, error }: { loading: boolean; error: string }) {
  if (loading)
    return (
      <div className="flex items-center justify-center gap-2.5 py-16 text-slate-400">
        <Spinner width={18} height={18} /> 加载中…
      </div>
    );
  if (error) return <div className="py-16 text-center text-rose-500">加载失败：{error}</div>;
  return null;
}

// ---------------------------------------------------------------------------
// 概览
// ---------------------------------------------------------------------------

function Overview({ token }: { token: string }) {
  const { data, loading, error } = useAdminData<AdminStats>(() => admin.stats(token), [token]);
  if (loading || error) return <StateLine loading={loading} error={error} />;
  const s = data!;
  const errRate = s.calls_24h ? ((s.calls_errors_24h / s.calls_24h) * 100).toFixed(1) : "0.0";
  const cards = [
    { label: "用户", value: s.users, sub: `凭据 ${s.secrets} 条` },
    { label: "技能", value: s.skills, sub: `已公开 ${s.skills_public} 个` },
    { label: "MCP 服务", value: s.mcps, sub: `已公开 ${s.mcps_public} 个` },
    { label: "24h 调用", value: s.calls_24h, sub: `错误 ${s.calls_errors_24h} · ${errRate}%` },
    { label: "累计调用", value: s.calls_total, sub: `资源包 ${s.blobs} · ${humanSize(s.blobs_bytes)}` },
  ];
  const maxCount = Math.max(1, ...s.top_tools.map((t) => t.count));

  return (
    <div className="space-y-6">
      <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-5">
        {cards.map((c) => (
          <div
            key={c.label}
            className="rounded-2xl border border-slate-200/70 bg-white p-5 shadow-sm transition hover:-translate-y-0.5 hover:shadow-md"
          >
            <div className="text-sm text-slate-400">{c.label}</div>
            <div className="mt-2 bg-gradient-to-br from-slate-800 to-slate-600 bg-clip-text text-3xl font-bold text-transparent">
              {c.value}
            </div>
            <div className="mt-2 text-xs text-slate-400">{c.sub}</div>
          </div>
        ))}
      </div>

      <Panel>
        <h2 className="mb-5 font-bold text-slate-800">24 小时热门工具</h2>
        {s.top_tools.length === 0 ? (
          <div className="py-8 text-center text-sm text-slate-400">最近 24 小时暂无工具调用。</div>
        ) : (
          <div className="space-y-3">
            {s.top_tools.map((t, i) => (
              <div key={t.tool} className="flex items-center gap-4">
                <span className="w-5 text-right text-sm text-slate-400">{i + 1}</span>
                <span className="w-48 flex-none truncate rounded-md bg-slate-100 px-2 py-0.5 font-mono text-xs text-slate-600">
                  {t.tool}
                </span>
                <div className="h-2.5 flex-1 overflow-hidden rounded-full bg-slate-100">
                  <div
                    className="h-full rounded-full bg-gradient-to-r from-indigo-500 to-sky-400"
                    style={{ width: `${(t.count / maxCount) * 100}%` }}
                  />
                </div>
                <span className="w-10 text-right text-sm font-semibold text-slate-700">{t.count}</span>
              </div>
            ))}
          </div>
        )}
      </Panel>

      <Panel>
        <h2 className="mb-5 font-bold text-slate-800">最近错误</h2>
        {s.recent_errors.length === 0 ? (
          <div className="py-8 text-center text-sm text-slate-400">暂无错误记录。</div>
        ) : (
          <div className="space-y-3">
            {s.recent_errors.map((e, i) => (
              <div key={i} className="rounded-xl border border-slate-200 bg-slate-50/60 px-4 py-3">
                <div className="flex items-center justify-between gap-3">
                  <span className="truncate rounded-md bg-white px-2 py-0.5 font-mono text-xs text-slate-600">
                    {e.tool}
                  </span>
                  <span className="flex-none text-xs text-slate-400">{e.at}</span>
                </div>
                <div className="mt-1 text-xs text-slate-400">by {e.caller || "—"}</div>
                <div className="mt-1.5 font-mono text-xs text-rose-500">{e.error}</div>
              </div>
            ))}
          </div>
        )}
      </Panel>
    </div>
  );
}

// ---------------------------------------------------------------------------
// 表格
// ---------------------------------------------------------------------------

function Badge({ kind, children }: { kind: "public" | "private" | "ok" | "err"; children: React.ReactNode }) {
  const styles: Record<string, string> = {
    public: "border-emerald-200 bg-emerald-50 text-emerald-600",
    private: "border-amber-200 bg-amber-50 text-amber-600",
    ok: "border-emerald-200 bg-emerald-50 text-emerald-600",
    err: "border-rose-200 bg-rose-50 text-rose-500",
  };
  return (
    <span className={`rounded-md border px-1.5 py-0.5 text-xs font-medium ${styles[kind]}`}>{children}</span>
  );
}

function Table({ head, children }: { head: React.ReactNode[]; children: React.ReactNode }) {
  return (
    <Panel className="overflow-x-auto p-0">
      <table className="w-full text-left text-sm">
        <thead>
          <tr className="border-b border-slate-200 text-xs uppercase tracking-wide text-slate-400">
            {head.map((h, i) => (
              <th key={i} className="px-5 py-3 font-medium">
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody className="divide-y divide-slate-100">{children}</tbody>
      </table>
    </Panel>
  );
}

// ---------------------------------------------------------------------------
// 共享：表头操作区、行内按钮、表单控件、分组可见性编辑器
// ---------------------------------------------------------------------------

function Toolbar({ title, hint, action }: { title: string; hint?: string; action?: React.ReactNode }) {
  return (
    <div className="mb-4 flex items-end justify-between gap-4">
      <div>
        <h2 className="font-bold text-slate-800">{title}</h2>
        {hint && <p className="mt-1 text-sm text-slate-400">{hint}</p>}
      </div>
      {action}
    </div>
  );
}

function PrimaryButton({
  onClick,
  children,
}: {
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      className="flex items-center gap-1.5 rounded-xl bg-indigo-500 px-4 py-2 text-sm font-semibold text-white transition hover:bg-indigo-600"
    >
      {children}
    </button>
  );
}

function RowActions({ children }: { children: React.ReactNode }) {
  return <div className="flex justify-end gap-2">{children}</div>;
}

function MiniButton({
  onClick,
  tone = "indigo",
  children,
}: {
  onClick: () => void;
  tone?: "indigo" | "rose";
  children: React.ReactNode;
}) {
  const styles =
    tone === "rose"
      ? "border-rose-200 text-rose-500 hover:bg-rose-50"
      : "border-indigo-200 text-indigo-500 hover:bg-indigo-50";
  return (
    <button
      onClick={onClick}
      className={`flex items-center gap-1 rounded-lg border px-2.5 py-1.5 text-xs font-semibold transition ${styles}`}
    >
      {children}
    </button>
  );
}

const inputCls =
  "w-full rounded-xl border border-slate-200 bg-white px-3.5 py-2.5 text-sm outline-none transition focus:border-indigo-300 focus:ring-4 focus:ring-indigo-500/10";

const adminLabelCls = "mb-1.5 block text-xs font-semibold text-slate-600";

/** 编辑弹窗通用底部：取消 + 保存。 */
function ModalFooter({
  busy,
  onClose,
  onSave,
}: {
  busy: boolean;
  onClose: () => void;
  onSave: () => void;
}) {
  return (
    <>
      <button
        onClick={onClose}
        className="rounded-xl border border-slate-200 px-4 py-2 text-sm font-semibold text-slate-600 transition hover:bg-slate-50"
      >
        取消
      </button>
      <button
        onClick={onSave}
        disabled={busy}
        className="rounded-xl bg-indigo-500 px-4 py-2 text-sm font-semibold text-white transition hover:bg-indigo-600 disabled:opacity-60"
      >
        {busy ? "保存中…" : "保存"}
      </button>
    </>
  );
}

/** 逗号 / 换行分隔的列表解析（去空白、去空项）。 */
function parseCsv(text: string): string[] {
  return text
    .split(/[,\n]/)
    .map((s) => s.trim())
    .filter(Boolean);
}

/** 每行 KEY=VALUE 文本 → 映射。 */
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

/** 映射 → 每行 KEY=VALUE 文本。 */
function kvToText(kv?: Record<string, string>): string {
  return Object.entries(kv ?? {})
    .map(([k, v]) => `${k}=${v}`)
    .join("\n");
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1.5 block text-sm font-medium text-slate-600">{label}</span>
      {children}
    </label>
  );
}

/** 拉取分组列表（一次）。 */
function useGroups(token: string) {
  const [groups, setGroups] = useState<AdminGroup[]>([]);
  useEffect(() => {
    admin.groups(token).then(setGroups).catch(() => setGroups([]));
  }, [token]);
  return groups;
}

/** 拉取标签列表（一次）。 */
function useLabels(token: string) {
  const [labels, setLabels] = useState<AdminLabel[]>([]);
  useEffect(() => {
    admin.labels(token).then(setLabels).catch(() => setLabels([]));
  }, [token]);
  return labels;
}

/** 资源已分配标签的徽章行。 */
function LabelBadges({ labels }: { labels: { id: number; name: string }[] }) {
  if (labels.length === 0) return <span className="text-xs text-slate-300">—</span>;
  return (
    <div className="flex flex-wrap gap-1">
      {labels.map((l) => (
        <span
          key={l.id}
          className="rounded-md border border-slate-200 bg-slate-50 px-2 py-0.5 text-xs text-slate-600"
        >
          {l.name}
        </span>
      ))}
    </div>
  );
}

/** 把存储的 group_visibility 字符串渲染成简短标签。 */
function groupVisLabel(raw: string, groups: AdminGroup[]): string {
  const v = parseGroupVisibility(raw);
  if (v === "all") return "所有分组";
  if (v.length === 0) return "仅作者";
  const names = v.map((id) => groups.find((g) => g.id === id)?.name ?? `#${id}`);
  return names.join("、");
}

function GroupVisibilityEditor({
  value,
  groups,
  onChange,
}: {
  value: GroupVisibility;
  groups: AdminGroup[];
  onChange: (v: GroupVisibility) => void;
}) {
  const all = value === "all";
  const selected = Array.isArray(value) ? value : [];
  const toggle = (id: number) =>
    onChange(selected.includes(id) ? selected.filter((x) => x !== id) : [...selected, id]);
  return (
    <div className="space-y-2.5">
      <div className="flex gap-4 text-sm">
        <label className="flex items-center gap-2">
          <input type="radio" checked={all} onChange={() => onChange("all")} />
          所有分组可见
        </label>
        <label className="flex items-center gap-2">
          <input type="radio" checked={!all} onChange={() => onChange([])} />
          指定分组
        </label>
      </div>
      {!all &&
        (groups.length === 0 ? (
          <p className="rounded-lg bg-amber-50 px-3 py-2 text-xs text-amber-600">
            还没有分组，请先在「分组」中创建。未选择任何分组时仅作者本人可见。
          </p>
        ) : (
          <div className="flex flex-wrap gap-2">
            {groups.map((g) => {
              const on = selected.includes(g.id);
              return (
                <button
                  key={g.id}
                  type="button"
                  onClick={() => toggle(g.id)}
                  className={`rounded-full border px-3 py-1.5 text-xs font-medium transition ${
                    on
                      ? "border-indigo-300 bg-indigo-50 text-indigo-600"
                      : "border-slate-200 bg-white text-slate-500 hover:bg-slate-50"
                  }`}
                >
                  {g.name}
                </button>
              );
            })}
          </div>
        ))}
    </div>
  );
}

function ModalError({ error }: { error: string }) {
  if (!error) return null;
  return <div className="mt-3 text-sm text-rose-500">{error}</div>;
}



// ---------------------------------------------------------------------------
// 资源管理：技能 + MCP 统一视图（搜索 / 过滤 / 分页 / 多选 / 批量配置）
// ---------------------------------------------------------------------------

type ResourceKind = "skill" | "mcp";
const RESOURCES_PAGE_SIZE = 12;

/** 资源行的统一视图：技能与 MCP 归一为同一组字段，供表格与批量操作复用。 */
interface ResourceRow {
  kind: ResourceKind;
  owner: string;
  name: string;
  visibility: string;
  group_visibility: string;
  labels: { id: number; name: string }[];
  meta: string; // 技能=分类，MCP=运行时/协议
  likes: number;
  favorites: number;
  /** 下载次数（技能专属；MCP 为 null）。 */
  downloads: number | null;
  updated_at: string;
  raw: AdminSkill | AdminMcp;
}

function toRow(kind: ResourceKind, r: AdminSkill | AdminMcp): ResourceRow {
  const common = {
    kind,
    owner: r.owner,
    name: r.name,
    visibility: r.visibility,
    group_visibility: r.group_visibility,
    labels: r.labels,
    likes: r.likes,
    favorites: r.favorites,
    updated_at: r.updated_at,
    raw: r,
  };
  if (kind === "skill") {
    const s = r as AdminSkill;
    return { ...common, meta: s.category, downloads: s.downloads };
  }
  const m = r as AdminMcp;
  return { ...common, meta: `${m.runtime} · ${m.protocol}`, downloads: null };
}

function ResourcesManager({ token, notify }: { token: string; notify: (m: string) => void }) {
  const [kind, setKind] = useState<ResourceKind>("skill");
  const groups = useGroups(token);
  const labels = useLabels(token);

  const { data, loading, error, reload } = useAdminData<ResourceRow[]>(
    () =>
      kind === "skill"
        ? admin.skills(token).then((rs) => rs.map((r) => toRow("skill", r)))
        : admin.mcps(token).then((rs) => rs.map((r) => toRow("mcp", r))),
    [token, kind],
  );

  // 过滤条件。
  const [q, setQ] = useState("");
  const [visFilter, setVisFilter] = useState("");
  const [labelFilter, setLabelFilter] = useState<number | "">("");
  const [page, setPage] = useState(0);
  // 选中集合：key = owner/name。
  const [selected, setSelected] = useState<Set<string>>(new Set());
  // 弹窗。
  const [batch, setBatch] = useState(false);
  const [batchTransfer, setBatchTransfer] = useState(false);
  const [visEdit, setVisEdit] = useState<ResourceRow | null>(null);
  const [content, setContent] = useState<ResourceRow | null>(null);

  // 切换类型 / 过滤时复位分页与选择。
  useEffect(() => {
    setPage(0);
    setSelected(new Set());
  }, [kind, q, visFilter, labelFilter]);

  const all = data ?? [];
  const filtered = all.filter((r) => {
    if (q) {
      const needle = q.toLowerCase();
      if (!r.name.toLowerCase().includes(needle) && !r.owner.toLowerCase().includes(needle))
        return false;
    }
    if (visFilter && r.visibility !== visFilter) return false;
    if (labelFilter !== "" && !r.labels.some((l) => l.id === labelFilter)) return false;
    return true;
  });
  const lastPage = Math.max(0, Math.ceil(filtered.length / RESOURCES_PAGE_SIZE) - 1);
  const pageRows = filtered.slice(page * RESOURCES_PAGE_SIZE, (page + 1) * RESOURCES_PAGE_SIZE);

  const key = (r: ResourceRow) => `${r.owner}/${r.name}`;
  const pageAllSelected = pageRows.length > 0 && pageRows.every((r) => selected.has(key(r)));
  const toggleOne = (r: ResourceRow) =>
    setSelected((cur) => {
      const next = new Set(cur);
      if (next.has(key(r))) next.delete(key(r));
      else next.add(key(r));
      return next;
    });
  const togglePage = () =>
    setSelected((cur) => {
      const next = new Set(cur);
      if (pageAllSelected) pageRows.forEach((r) => next.delete(key(r)));
      else pageRows.forEach((r) => next.add(key(r)));
      return next;
    });

  const selectedTargets = all
    .filter((r) => selected.has(key(r)))
    .map((r) => ({ owner: r.owner, name: r.name }));

  async function delOne(r: ResourceRow) {
    const kindLabel = r.kind === "skill" ? "技能" : "MCP";
    if (!confirm(`删除${kindLabel}「${r.owner}/${r.name}」？此操作不可恢复。`)) return;
    try {
      if (r.kind === "skill") await admin.deleteSkill(token, r.owner, r.name);
      else await admin.deleteMcp(token, r.owner, r.name);
      notify("已删除");
      reload();
    } catch (e) {
      notify((e as Error).message);
    }
  }

  async function batchDelete() {
    const kindLabel = kind === "skill" ? "技能" : "MCP";
    if (!confirm(`删除选中的 ${selectedTargets.length} 个${kindLabel}？此操作不可恢复。`)) return;
    const results = await Promise.allSettled(
      selectedTargets.map((t) =>
        kind === "skill"
          ? admin.deleteSkill(token, t.owner, t.name)
          : admin.deleteMcp(token, t.owner, t.name),
      ),
    );
    const ok = results.filter((r) => r.status === "fulfilled").length;
    const fail = results.length - ok;
    notify(fail ? `删除 ${ok} 个，失败 ${fail} 个` : `已删除 ${ok} 个`);
    setSelected(new Set());
    reload();
  }

  return (
    <div className="space-y-4">
      {/* 类型切换 + 搜索 + 过滤 */}
      <div className="flex flex-wrap items-center gap-3">
        <div className="flex rounded-xl border border-slate-200 bg-white p-1 shadow-sm">
          {(["skill", "mcp"] as ResourceKind[]).map((k) => (
            <button
              key={k}
              onClick={() => setKind(k)}
              className={`rounded-lg px-4 py-1.5 text-sm font-semibold transition ${
                kind === k ? "bg-indigo-500 text-white" : "text-slate-500 hover:bg-slate-100"
              }`}
            >
              {k === "skill" ? "技能" : "MCP 服务"}
            </button>
          ))}
        </div>
        <input
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder="搜索名称 / 作者…"
          className={`${inputCls} max-w-[220px]`}
        />
        <select
          value={visFilter}
          onChange={(e) => setVisFilter(e.target.value)}
          className={`${selectCls} max-w-[150px]`}
          style={caretBg}
        >
          <option value="">全部可见性</option>
          <option value="public">public</option>
          <option value="private">private</option>
        </select>
        <select
          value={labelFilter}
          onChange={(e) => setLabelFilter(e.target.value ? Number(e.target.value) : "")}
          className={`${selectCls} max-w-[160px]`}
          style={caretBg}
        >
          <option value="">全部标签</option>
          {labels.map((l) => (
            <option key={l.id} value={l.id}>
              {l.name}
            </option>
          ))}
        </select>
        <div className="flex-1" />
        <span className="text-sm text-slate-400">
          共 <span className="font-semibold text-slate-600">{filtered.length}</span> 个
        </span>
      </div>

      {/* 批量操作条 */}
      {selected.size > 0 && (
        <div className="flex flex-wrap items-center gap-3 rounded-xl border border-indigo-200 bg-indigo-50/70 px-4 py-2.5">
          <span className="text-sm font-semibold text-indigo-700">已选 {selected.size} 个</span>
          <div className="flex-1" />
          <MiniButton onClick={() => setBatch(true)}>批量配置</MiniButton>
          <MiniButton onClick={() => setBatchTransfer(true)}>批量转移</MiniButton>
          <MiniButton tone="rose" onClick={batchDelete}>
            批量删除
          </MiniButton>
          <button
            onClick={() => setSelected(new Set())}
            className="text-xs font-medium text-slate-400 hover:text-slate-600"
          >
            取消选择
          </button>
        </div>
      )}

      {loading || error ? (
        <StateLine loading={loading} error={error} />
      ) : filtered.length === 0 ? (
        <Panel>没有匹配的资源。</Panel>
      ) : (
        <>
          <Table
            head={[
              <input
                key="chk"
                type="checkbox"
                checked={pageAllSelected}
                onChange={togglePage}
                className="cursor-pointer"
              />,
              kind === "skill" ? "技能" : "MCP",
              kind === "skill" ? "分类" : "运行时",
              "可见性",
              "可见分组",
              "标签",
              "互动",
              "更新于",
              "操作",
            ]}
          >
            {pageRows.map((r) => (
              <tr key={key(r)} className="text-slate-700">
                <td className="px-5 py-3">
                  <input
                    type="checkbox"
                    checked={selected.has(key(r))}
                    onChange={() => toggleOne(r)}
                    className="cursor-pointer"
                  />
                </td>
                <td className="px-5 py-3">
                  <div className="font-semibold text-slate-800">{r.name}</div>
                  <div className="text-xs text-slate-400">@{r.owner}</div>
                </td>
                <td className="px-5 py-3 text-slate-500">{r.meta}</td>
                <td className="px-5 py-3">
                  <Badge kind={r.visibility === "public" ? "public" : "private"}>
                    {r.visibility}
                  </Badge>
                </td>
                <td className="px-5 py-3 text-xs text-slate-500">
                  {r.visibility === "public" ? groupVisLabel(r.group_visibility, groups) : "—"}
                </td>
                <td className="px-5 py-3">
                  <LabelBadges labels={r.labels} />
                </td>
                <td className="whitespace-nowrap px-5 py-3 text-xs text-slate-500">
                  ♥ {r.likes} · ★ {r.favorites}
                  {r.downloads != null && <> · ⬇ {r.downloads}</>}
                </td>
                <td className="px-5 py-3 text-xs text-slate-400">{r.updated_at}</td>
                <td className="px-5 py-3">
                  <RowActions>
                    <MiniButton onClick={() => setContent(r)}>编辑</MiniButton>
                    <MiniButton onClick={() => setVisEdit(r)}>配置</MiniButton>
                    <MiniButton tone="rose" onClick={() => delOne(r)}>
                      <TrashIcon width={13} height={13} />
                    </MiniButton>
                  </RowActions>
                </td>
              </tr>
            ))}
          </Table>

          {lastPage > 0 && (
            <div className="flex items-center justify-end gap-2">
              <span className="mr-2 text-sm text-slate-400">
                第 {page + 1} / {lastPage + 1} 页
              </span>
              <button
                onClick={() => setPage((p) => Math.max(0, p - 1))}
                disabled={page <= 0}
                className="rounded-lg border border-slate-200 px-3 py-1.5 text-sm font-medium text-slate-600 transition hover:bg-slate-50 disabled:cursor-not-allowed disabled:opacity-40"
              >
                上一页
              </button>
              <button
                onClick={() => setPage((p) => Math.min(lastPage, p + 1))}
                disabled={page >= lastPage}
                className="rounded-lg border border-slate-200 px-3 py-1.5 text-sm font-medium text-slate-600 transition hover:bg-slate-50 disabled:cursor-not-allowed disabled:opacity-40"
              >
                下一页
              </button>
            </div>
          )}
        </>
      )}

      {batch && (
        <BatchModal
          kind={kind}
          token={token}
          targets={selectedTargets}
          groups={groups}
          labels={labels}
          onClose={() => setBatch(false)}
          onSaved={(msg) => {
            setBatch(false);
            setSelected(new Set());
            notify(msg);
            reload();
          }}
        />
      )}
      {batchTransfer && (
        <BatchTransferModal
          kind={kind}
          token={token}
          targets={selectedTargets}
          onClose={() => setBatchTransfer(false)}
          onSaved={(msg) => {
            setBatchTransfer(false);
            setSelected(new Set());
            notify(msg);
            reload();
          }}
        />
      )}
      {visEdit && (
        <VisibilityModal
          kind={visEdit.kind}
          token={token}
          owner={visEdit.owner}
          name={visEdit.name}
          visibility={visEdit.visibility}
          groupVisibilityRaw={visEdit.group_visibility}
          groups={groups}
          labels={labels}
          currentLabelIds={visEdit.labels.map((l) => l.id)}
          onClose={() => setVisEdit(null)}
          onSaved={() => {
            setVisEdit(null);
            notify("已更新");
            reload();
          }}
        />
      )}
      {content && content.kind === "skill" && (
        <AdminSkillEditModal
          token={token}
          skill={content.raw as AdminSkill}
          onClose={() => setContent(null)}
          onChanged={reload}
          onSaved={() => {
            setContent(null);
            notify("已更新");
            reload();
          }}
        />
      )}
      {content && content.kind === "mcp" && (
        <AdminMcpEditModal
          token={token}
          mcp={content.raw as AdminMcp}
          onClose={() => setContent(null)}
          onSaved={() => {
            setContent(null);
            notify("已更新");
            reload();
          }}
        />
      )}
    </div>
  );
}

/** 批量配置：对选中资源统一改可见性 / 可见分组 / 增删标签。各项可独立开关。 */
function BatchModal({
  kind,
  token,
  targets,
  groups,
  labels,
  onClose,
  onSaved,
}: {
  kind: ResourceKind;
  token: string;
  targets: { owner: string; name: string }[];
  groups: AdminGroup[];
  labels: AdminLabel[];
  onClose: () => void;
  onSaved: (msg: string) => void;
}) {
  const [visOn, setVisOn] = useState(false);
  const [visibility, setVisibility] = useState("public");
  const [gvOn, setGvOn] = useState(false);
  const [gv, setGv] = useState<GroupVisibility>("all");
  const [addLabelIds, setAddLabelIds] = useState<number[]>([]);
  const [removeLabelIds, setRemoveLabelIds] = useState<number[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const toggle = (arr: number[], id: number) =>
    arr.includes(id) ? arr.filter((x) => x !== id) : [...arr, id];

  const nothingToDo =
    !visOn && !gvOn && addLabelIds.length === 0 && removeLabelIds.length === 0;

  async function save() {
    if (nothingToDo) {
      setError("请至少选择一项要修改的配置");
      return;
    }
    setBusy(true);
    setError("");
    try {
      const res = await admin.batchUpdate(token, {
        kind,
        targets,
        visibility: visOn ? visibility : undefined,
        group_visibility: gvOn ? gv : undefined,
        add_label_ids: addLabelIds,
        remove_label_ids: removeLabelIds,
      });
      const failMsg = res.failed.length ? `，失败 ${res.failed.length} 个` : "";
      onSaved(`已更新 ${res.updated} 个${failMsg}`);
    } catch (e) {
      setError((e as Error).message);
      setBusy(false);
    }
  }

  return (
    <Modal
      title="批量配置"
      subtitle={`对选中的 ${targets.length} 个${kind === "skill" ? "技能" : "MCP"}生效`}
      onClose={onClose}
      footer={<ModalFooter busy={busy} onClose={onClose} onSave={save} />}
    >
      <div className="space-y-5">
        {/* 可见性 */}
        <div className="rounded-xl border border-slate-200 p-4">
          <label className="flex items-center gap-2 text-sm font-semibold text-slate-700">
            <input type="checkbox" checked={visOn} onChange={(e) => setVisOn(e.target.checked)} />
            设置可见性
          </label>
          {visOn && (
            <div className="mt-3 flex gap-4 text-sm">
              <label className="flex items-center gap-2">
                <input
                  type="radio"
                  checked={visibility === "public"}
                  onChange={() => setVisibility("public")}
                />
                public（上架市场）
              </label>
              <label className="flex items-center gap-2">
                <input
                  type="radio"
                  checked={visibility === "private"}
                  onChange={() => setVisibility("private")}
                />
                private（仅作者）
              </label>
            </div>
          )}
        </div>

        {/* 可见分组 */}
        <div className="rounded-xl border border-slate-200 p-4">
          <label className="flex items-center gap-2 text-sm font-semibold text-slate-700">
            <input type="checkbox" checked={gvOn} onChange={(e) => setGvOn(e.target.checked)} />
            设置可见分组
          </label>
          {gvOn && (
            <div className="mt-3">
              <GroupVisibilityEditor value={gv} groups={groups} onChange={setGv} />
            </div>
          )}
        </div>

        {/* 标签增删 */}
        <div className="rounded-xl border border-slate-200 p-4">
          <div className="text-sm font-semibold text-slate-700">标签</div>
          {labels.length === 0 ? (
            <p className="mt-2 rounded-lg bg-amber-50 px-3 py-2 text-xs text-amber-600">
              还没有标签，请先在「标签」中创建。
            </p>
          ) : (
            <div className="mt-3 space-y-3">
              <div>
                <div className="mb-1.5 text-xs text-slate-400">添加这些标签</div>
                <div className="flex flex-wrap gap-2">
                  {labels.map((l) => {
                    const on = addLabelIds.includes(l.id);
                    return (
                      <button
                        key={l.id}
                        type="button"
                        onClick={() => setAddLabelIds((c) => toggle(c, l.id))}
                        className={`rounded-full border px-3 py-1.5 text-xs font-medium transition ${
                          on
                            ? "border-emerald-300 bg-emerald-50 text-emerald-600"
                            : "border-slate-200 bg-white text-slate-500 hover:bg-slate-50"
                        }`}
                      >
                        {on ? "+ " : ""}
                        {l.name}
                      </button>
                    );
                  })}
                </div>
              </div>
              <div>
                <div className="mb-1.5 text-xs text-slate-400">移除这些标签</div>
                <div className="flex flex-wrap gap-2">
                  {labels.map((l) => {
                    const on = removeLabelIds.includes(l.id);
                    return (
                      <button
                        key={l.id}
                        type="button"
                        onClick={() => setRemoveLabelIds((c) => toggle(c, l.id))}
                        className={`rounded-full border px-3 py-1.5 text-xs font-medium transition ${
                          on
                            ? "border-rose-300 bg-rose-50 text-rose-500"
                            : "border-slate-200 bg-white text-slate-500 hover:bg-slate-50"
                        }`}
                      >
                        {on ? "− " : ""}
                        {l.name}
                      </button>
                    );
                  })}
                </div>
              </div>
            </div>
          )}
        </div>
        <ModalError error={error} />
      </div>
    </Modal>
  );
}

/** 批量转移：把选中的技能 / MCP 统一转给另一个用户。重名的逐条报错，不阻断其余。 */
function BatchTransferModal({
  kind,
  token,
  targets,
  onClose,
  onSaved,
}: {
  kind: ResourceKind;
  token: string;
  targets: { owner: string; name: string }[];
  onClose: () => void;
  onSaved: (msg: string) => void;
}) {
  const [target, setTarget] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [failed, setFailed] = useState<{ owner: string; name: string; error: string }[]>([]);
  const kindLabel = kind === "skill" ? "技能" : "MCP";

  async function save() {
    const t = target.trim();
    if (!t) {
      setError("请输入接收方用户名");
      return;
    }
    if (!confirm(`确认把选中的 ${targets.length} 个${kindLabel}转移给「${t}」？`)) return;
    setBusy(true);
    setError("");
    setFailed([]);
    try {
      const res = await admin.transferResources(token, { kind, targets, to_username: t });
      if (res.failed.length) {
        setFailed(res.failed);
        setError(`已转移 ${res.updated} 个，失败 ${res.failed.length} 个`);
        setBusy(false);
      } else {
        onSaved(`已把 ${res.updated} 个${kindLabel}转移给 ${t}`);
      }
    } catch (e) {
      setError((e as Error).message);
      setBusy(false);
    }
  }

  return (
    <Modal
      title="批量转移"
      subtitle={`把选中的 ${targets.length} 个${kindLabel}转移给另一个用户`}
      onClose={onClose}
      footer={<ModalFooter busy={busy} onClose={onClose} onSave={save} />}
    >
      <div className="space-y-4">
        <Field label="接收方用户名">
          <input
            autoFocus
            value={target}
            onChange={(e) => setTarget(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && save()}
            placeholder="必须是已存在的用户"
            className={inputCls}
          />
        </Field>
        <p className="rounded-lg bg-amber-50 px-3 py-2 text-xs text-amber-600">
          转移后资源归接收方所有；与接收方既有资源重名的会失败并保留在原账号名下。
        </p>
        {failed.length > 0 && (
          <ul className="max-h-40 list-inside list-disc overflow-auto rounded-lg bg-rose-50 px-3 py-2 text-xs text-rose-500">
            {failed.map((f, i) => (
              <li key={i}>
                {f.owner}/{f.name}：{f.error}
              </li>
            ))}
          </ul>
        )}
        <ModalError error={error} />
      </div>
    </Modal>
  );
}

/** 技能 / MCP 的可见性 + 分组可见性 + 标签配置弹窗。 */
function VisibilityModal({
  kind,
  token,
  owner,
  name,
  visibility: initVis,
  groupVisibilityRaw,
  groups,
  labels,
  currentLabelIds,
  onClose,
  onSaved,
}: {
  kind: "skill" | "mcp";
  token: string;
  owner: string;
  name: string;
  visibility: string;
  groupVisibilityRaw: string;
  groups: AdminGroup[];
  labels: AdminLabel[];
  currentLabelIds: number[];
  onClose: () => void;
  onSaved: () => void;
}) {
  const [visibility, setVisibility] = useState(initVis);
  const [gv, setGv] = useState<GroupVisibility>(parseGroupVisibility(groupVisibilityRaw));
  const [labelIds, setLabelIds] = useState<number[]>(currentLabelIds);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const toggleLabel = (id: number) =>
    setLabelIds((cur) => (cur.includes(id) ? cur.filter((x) => x !== id) : [...cur, id]));

  async function save() {
    setBusy(true);
    setError("");
    try {
      const body = { visibility, group_visibility: gv, label_ids: labelIds };
      if (kind === "skill") await admin.updateSkill(token, owner, name, body);
      else await admin.updateMcp(token, owner, name, body);
      onSaved();
    } catch (e) {
      setError((e as Error).message);
      setBusy(false);
    }
  }

  return (
    <Modal
      title="配置资源"
      subtitle={`${owner}/${name}`}
      onClose={onClose}
      footer={
        <>
          <button
            onClick={onClose}
            className="rounded-xl border border-slate-200 px-4 py-2 text-sm font-semibold text-slate-600 transition hover:bg-slate-50"
          >
            取消
          </button>
          <button
            onClick={save}
            disabled={busy}
            className="rounded-xl bg-indigo-500 px-4 py-2 text-sm font-semibold text-white transition hover:bg-indigo-600 disabled:opacity-60"
          >
            {busy ? "保存中…" : "保存"}
          </button>
        </>
      }
    >
      <div className="space-y-5">
        <Field label="可见性">
          <div className="flex gap-4 text-sm">
            <label className="flex items-center gap-2">
              <input
                type="radio"
                checked={visibility === "public"}
                onChange={() => setVisibility("public")}
              />
              public（上架市场）
            </label>
            <label className="flex items-center gap-2">
              <input
                type="radio"
                checked={visibility === "private"}
                onChange={() => setVisibility("private")}
              />
              private（仅作者）
            </label>
          </div>
        </Field>
        {visibility === "public" && (
          <Field label="可见分组">
            <GroupVisibilityEditor value={gv} groups={groups} onChange={setGv} />
          </Field>
        )}
        <Field label="标签">
          {labels.length === 0 ? (
            <p className="rounded-lg bg-amber-50 px-3 py-2 text-xs text-amber-600">
              还没有标签，请先在「标签」中创建。
            </p>
          ) : (
            <div className="flex flex-wrap gap-2">
              {labels.map((l) => {
                const on = labelIds.includes(l.id);
                return (
                  <button
                    key={l.id}
                    type="button"
                    onClick={() => toggleLabel(l.id)}
                    className={`rounded-full border px-3 py-1.5 text-xs font-medium transition ${
                      on
                        ? "border-indigo-300 bg-indigo-50 text-indigo-600"
                        : "border-slate-200 bg-white text-slate-500 hover:bg-slate-50"
                    }`}
                  >
                    {l.name}
                  </button>
                );
              })}
            </div>
          )}
        </Field>
        <ModalError error={error} />
      </div>
    </Modal>
  );
}

/** 管理员：编辑技能内容（分类 / 版本 / 描述 / 标签 / 依赖 / SKILL.md）。 */
function AdminSkillEditModal({
  token,
  skill,
  onClose,
  onSaved,
  onChanged,
}: {
  token: string;
  skill: AdminSkill;
  onClose: () => void;
  onSaved: () => void;
  /** 版本删除等就地变更后通知外层刷新列表（弹窗保持打开）。 */
  onChanged?: () => void;
}) {
  const [version, setVersion] = useState(skill.version);
  const [category, setCategory] = useState<SkillCategory>(skill.category as SkillCategory);
  const [description, setDescription] = useState(skill.description);
  const [tags, setTags] = useState(skill.tags.join(", "));
  const [mcpDeps, setMcpDeps] = useState(skill.mcp_dependencies.join(", "));
  const [preferred, setPreferred] = useState(skill.preferred_tools.join(", "));
  const [skillMd, setSkillMd] = useState(skill.skill_md);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  // 版本历史：null=加载中；head 为当前最新版本号（删除最新版后服务端会回退并回传新值）。
  const [versions, setVersions] = useState<SkillVersionInfo[] | null>(null);
  const [head, setHead] = useState(skill.version);
  const [busyVer, setBusyVer] = useState<string | null>(null);

  // 说明书文件名随分类而定：agent → AGENT.md，其余 → SKILL.md。
  const doc = docFilename(category);

  useEffect(() => {
    let alive = true;
    admin
      .skillVersions(token, skill.owner, skill.name)
      .then((list) => alive && setVersions(list))
      .catch(() => alive && setVersions([]));
    return () => {
      alive = false;
    };
  }, [token, skill.owner, skill.name]);

  async function removeVersion(v: SkillVersionInfo) {
    const isHead = v.version === head;
    const msg = isHead
      ? `删除最新版 v${v.version}？\n删除后次新版本将自动成为最新版（市场默认安装随之回退）。`
      : `删除版本 v${v.version}？\n该版本副本与压缩体将被清理，客户端将无法再安装此版本，不可恢复。`;
    if (!confirm(msg)) return;
    setBusyVer(v.version);
    setError("");
    try {
      const r = await admin.deleteSkillVersion(token, skill.owner, skill.name, v.version);
      setVersions(r.versions);
      setHead(r.head);
      // 版本输入框若未被手工改过（仍为旧最新版/被删版本），同步到新最新版，
      // 避免「保存」把已删除的版本号写回去。
      setVersion((cur) => (cur === head || cur === v.version ? r.head : cur));
      onChanged?.();
    } catch (e) {
      setError((e as Error).message);
    }
    setBusyVer(null);
  }

  async function save() {
    if (!skillMd.trim()) {
      setError(`${doc} 不能为空`);
      return;
    }
    setBusy(true);
    setError("");
    try {
      await admin.updateSkill(token, skill.owner, skill.name, {
        version: version.trim() || "0.1.0",
        category,
        description: description.trim(),
        tags: parseCsv(tags),
        mcp_dependencies: parseCsv(mcpDeps),
        preferred_tools: parseCsv(preferred),
        skill_md: skillMd,
      });
      onSaved();
    } catch (e) {
      setError((e as Error).message);
      setBusy(false);
    }
  }

  return (
    <Modal
      title="编辑技能内容"
      subtitle={`${skill.owner}/${skill.name}`}
      onClose={onClose}
      wide
      footer={<ModalFooter busy={busy} onClose={onClose} onSave={save} />}
    >
      <div className="space-y-4">
        <div className="grid grid-cols-3 gap-3">
          <div>
            <label className={adminLabelCls}>版本</label>
            <input className={inputCls} value={version} onChange={(e) => setVersion(e.target.value)} />
          </div>
          <div>
            <label className={adminLabelCls}>分类</label>
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
            <label className={adminLabelCls}>压缩体</label>
            <div className="px-1 py-2.5 text-sm text-slate-500">
              {skill.has_archive ? `📦 ${humanSize(skill.archive_size)}` : "纯文本"}
            </div>
          </div>
        </div>
        <div>
          <label className={adminLabelCls}>
            版本历史（删除最新版将自动回退到次新版本；仅剩一个版本时不可删除）
          </label>
          {versions === null ? (
            <div className="px-1 py-1 text-xs text-slate-400">加载中…</div>
          ) : versions.length === 0 ? (
            <div className="px-1 py-1 text-xs text-slate-400">无版本记录</div>
          ) : (
            <div className="divide-y divide-slate-100 rounded-lg border border-slate-200">
              {versions.map((v) => (
                <div key={v.version} className="flex items-center gap-3 px-3 py-1.5 text-xs">
                  <code className="font-mono font-semibold text-slate-700">v{v.version}</code>
                  {v.version === head && (
                    <span className="rounded border border-emerald-200 bg-emerald-50 px-1.5 text-emerald-600">
                      最新
                    </span>
                  )}
                  <span className="text-slate-400">
                    {v.archive_size > 0 ? humanSize(v.archive_size) : "纯文本"}
                  </span>
                  <span className="flex-1 text-slate-400">{v.created_at.replace(" UTC", "")}</span>
                  <button
                    onClick={() => removeVersion(v)}
                    disabled={versions.length <= 1 || busyVer !== null}
                    title={versions.length <= 1 ? "仅剩最后一个版本，如需下架请删除整个技能" : "删除该版本副本"}
                    className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-rose-500 transition hover:bg-rose-50 disabled:cursor-not-allowed disabled:opacity-40"
                  >
                    {busyVer === v.version ? <Spinner /> : <TrashIcon width={12} height={12} />}
                    删除
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>
        <div>
          <label className={adminLabelCls}>描述</label>
          <input
            className={inputCls}
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
        </div>
        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className={adminLabelCls}>标签（逗号/换行分隔）</label>
            <input className={inputCls} value={tags} onChange={(e) => setTags(e.target.value)} />
          </div>
          <div>
            <label className={adminLabelCls}>依赖 MCP（owner/name）</label>
            <input className={inputCls} value={mcpDeps} onChange={(e) => setMcpDeps(e.target.value)} />
          </div>
        </div>
        <div>
          <label className={adminLabelCls}>倾向工具（可选）</label>
          <input className={inputCls} value={preferred} onChange={(e) => setPreferred(e.target.value)} />
        </div>
        <div>
          <label className={adminLabelCls}>{doc}</label>
          <textarea
            className={`${inputCls} min-h-[180px] resize-y font-mono text-xs leading-relaxed`}
            value={skillMd}
            onChange={(e) => setSkillMd(e.target.value)}
          />
        </div>
        {skill.has_archive && (
          <p className="rounded-lg bg-amber-50 px-3 py-2 text-xs text-amber-600">
            该技能含压缩体：此处仅改写元数据与展示用的 {doc}，压缩包内文件不变。
          </p>
        )}
        <ModalError error={error} />
      </div>
    </Modal>
  );
}

/** 管理员：编辑 MCP 运行清单（描述 / 版本 / 运行拓扑 / URL 或命令 / 变量）。名称锁定不变。 */
function AdminMcpEditModal({
  token,
  mcp,
  onClose,
  onSaved,
}: {
  token: string;
  mcp: AdminMcp;
  onClose: () => void;
  onSaved: () => void;
}) {
  const m = mcp.manifest;
  const [version, setVersion] = useState(m.version);
  const [description, setDescription] = useState(m.description);
  const [runtime, setRuntime] = useState<Runtime>(m.runtime);
  const [protocol, setProtocol] = useState<Protocol>(m.protocol);
  const [target, setTarget] = useState(m.runtime === "local" ? (m.command ?? "") : (m.url ?? ""));
  const [kvText, setKvText] = useState(kvToText(m.runtime === "local" ? m.env : m.headers));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const local = runtime === "local";

  function onRuntimeChange(r: Runtime) {
    setRuntime(r);
    setProtocol(r === "local" ? "stdio" : "streamable");
  }

  async function save() {
    if (!target.trim()) {
      setError(local ? "请填写启动命令" : "请填写 URL");
      return;
    }
    const kv = parseKv(kvText);
    const manifest: McpManifest = {
      resource_type: "mcp",
      name: mcp.name,
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
    setError("");
    try {
      await admin.updateMcp(token, mcp.owner, mcp.name, { manifest });
      onSaved();
    } catch (e) {
      setError((e as Error).message);
      setBusy(false);
    }
  }

  return (
    <Modal
      title="编辑 MCP 清单"
      subtitle={`${mcp.owner}/${mcp.name}`}
      onClose={onClose}
      wide
      footer={<ModalFooter busy={busy} onClose={onClose} onSave={save} />}
    >
      <div className="space-y-4">
        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className={adminLabelCls}>名称（不可改）</label>
            <input className={`${inputCls} bg-slate-50 text-slate-400`} value={mcp.name} disabled />
          </div>
          <div>
            <label className={adminLabelCls}>版本</label>
            <input className={inputCls} value={version} onChange={(e) => setVersion(e.target.value)} />
          </div>
        </div>
        <div>
          <label className={adminLabelCls}>描述</label>
          <input
            className={inputCls}
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
        </div>
        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className={adminLabelCls}>运行时</label>
            <select
              className={inputCls}
              value={runtime}
              onChange={(e) => onRuntimeChange(e.target.value as Runtime)}
            >
              <option value="remote">remote</option>
              <option value="local">local</option>
            </select>
          </div>
          <div>
            <label className={adminLabelCls}>协议</label>
            <select
              className={inputCls}
              value={protocol}
              onChange={(e) => setProtocol(e.target.value as Protocol)}
            >
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
          <label className={adminLabelCls}>{local ? "启动命令" : "URL"}</label>
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
          <label className={adminLabelCls}>
            {local ? "环境变量（每行 KEY=VALUE，可选）" : "Headers（每行 KEY=VALUE，可选）"}
          </label>
          <textarea
            className={`${inputCls} min-h-[64px] resize-y font-mono text-xs`}
            value={kvText}
            placeholder={local ? "TOKEN={ACEMCP_TOKEN}" : "Authorization=Bearer {AIKO_HUB_KEY}"}
            onChange={(e) => setKvText(e.target.value)}
          />
        </div>
        <ModalError error={error} />
      </div>
    </Modal>
  );
}

function UsersTable({ token, notify }: { token: string; notify: (m: string) => void }) {
  const { data, loading, error, reload } = useAdminData<AdminUser[]>(
    () => admin.users(token),
    [token],
  );
  const groups = useGroups(token);
  const [modal, setModal] = useState<{ edit: AdminUser | null } | null>(null);
  const [transferFrom, setTransferFrom] = useState<AdminUser | null>(null);
  if (loading || error) return <StateLine loading={loading} error={error} />;
  const rows = data!;

  async function del(u: AdminUser) {
    if (
      !confirm(
        `删除用户「${u.username}」？\n其名下 ${u.skills} 个技能、${u.mcps} 个 MCP、${u.secrets} 条凭据将一并删除，不可恢复。`,
      )
    )
      return;
    try {
      await admin.deleteUser(token, u.id);
      notify("已删除");
      reload();
    } catch (e) {
      notify((e as Error).message);
    }
  }

  return (
    <div className="space-y-4">
      <Toolbar
        title="用户"
        hint="创建用户、调整分组归属、重置密码、转移名下资源或删除。"
        action={
          <PrimaryButton onClick={() => setModal({ edit: null })}>
            <PlusIcon width={16} height={16} /> 新建用户
          </PrimaryButton>
        }
      />
      <Table head={["用户名", "分组", "技能", "MCP", "凭据", "注册于", "操作"]}>
        {rows.map((u) => (
          <tr key={u.id} className="text-slate-700">
            <td className="px-5 py-3 font-semibold text-slate-800">{u.username}</td>
            <td className="px-5 py-3">
              {u.groups.length > 0 ? (
                <div className="flex flex-wrap gap-1">
                  {u.groups.map((g) => (
                    <span
                      key={g.id}
                      className="rounded-md border border-slate-200 bg-slate-50 px-2 py-0.5 text-xs text-slate-600"
                    >
                      {g.name}
                    </span>
                  ))}
                </div>
              ) : (
                <span className="text-xs text-slate-300">无</span>
              )}
            </td>
            <td className="px-5 py-3 text-slate-500">{u.skills}</td>
            <td className="px-5 py-3 text-slate-500">{u.mcps}</td>
            <td className="px-5 py-3 text-slate-500">{u.secrets}</td>
            <td className="px-5 py-3 text-xs text-slate-400">{u.created_at}</td>
            <td className="px-5 py-3">
              <RowActions>
                <MiniButton onClick={() => setModal({ edit: u })}>编辑</MiniButton>
                <MiniButton onClick={() => setTransferFrom(u)}>转移资源</MiniButton>
                <MiniButton tone="rose" onClick={() => del(u)}>
                  <TrashIcon width={13} height={13} />
                </MiniButton>
              </RowActions>
            </td>
          </tr>
        ))}
      </Table>
      {modal && (
        <UserModal
          token={token}
          edit={modal.edit}
          groups={groups}
          onClose={() => setModal(null)}
          onSaved={(msg) => {
            setModal(null);
            notify(msg);
            reload();
          }}
        />
      )}
      {transferFrom && (
        <UserTransferModal
          token={token}
          from={transferFrom}
          onClose={() => setTransferFrom(null)}
          onSaved={(msg) => {
            setTransferFrom(null);
            notify(msg);
            reload();
          }}
        />
      )}
    </div>
  );
}

/** 整户转移：把某用户名下全部技能与 MCP 转给另一个用户（注销前的资产交接）。 */
function UserTransferModal({
  token,
  from,
  onClose,
  onSaved,
}: {
  token: string;
  from: AdminUser;
  onClose: () => void;
  onSaved: (msg: string) => void;
}) {
  const [target, setTarget] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [skipped, setSkipped] = useState<string[]>([]);

  async function save() {
    const t = target.trim();
    if (!t) {
      setError("请输入接收方用户名");
      return;
    }
    if (
      !confirm(
        `确认把「${from.username}」名下的 ${from.skills} 个技能、${from.mcps} 个 MCP 全部转移给「${t}」？\n加密凭据属个人机密，不会转移。`,
      )
    )
      return;
    setBusy(true);
    setError("");
    setSkipped([]);
    try {
      const res = await admin.transferUser(token, from.id, t);
      if (res.skipped.length) {
        setSkipped(res.skipped);
        setError(
          `已转移技能 ${res.skills_moved} 个、MCP ${res.mcps_moved} 个；${res.skipped.length} 个因重名跳过`,
        );
        setBusy(false);
      } else {
        onSaved(`已把 ${from.username} 的技能 ${res.skills_moved} 个、MCP ${res.mcps_moved} 个转移给 ${t}`);
      }
    } catch (e) {
      setError((e as Error).message);
      setBusy(false);
    }
  }

  return (
    <Modal
      title="转移用户资源"
      subtitle={`把 ${from.username} 名下全部技能与 MCP 转给另一个用户`}
      onClose={onClose}
      footer={<ModalFooter busy={busy} onClose={onClose} onSave={save} />}
    >
      <div className="space-y-4">
        <Field label="接收方用户名">
          <input
            autoFocus
            value={target}
            onChange={(e) => setTarget(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && save()}
            placeholder="必须是已存在的用户"
            className={inputCls}
          />
        </Field>
        <p className="rounded-lg bg-amber-50 px-3 py-2 text-xs text-amber-600">
          适用于用户注销前的资产交接：技能与 MCP 归属整体变更；与接收方重名的资源会跳过并保留在原账号名下；
          加密凭据不随迁。
        </p>
        {skipped.length > 0 && (
          <ul className="max-h-40 list-inside list-disc overflow-auto rounded-lg bg-rose-50 px-3 py-2 text-xs text-rose-500">
            {skipped.map((s, i) => (
              <li key={i}>{s}</li>
            ))}
          </ul>
        )}
        <ModalError error={error} />
      </div>
    </Modal>
  );
}

function UserModal({
  token,
  edit,
  groups,
  onClose,
  onSaved,
}: {
  token: string;
  edit: AdminUser | null;
  groups: AdminGroup[];
  onClose: () => void;
  onSaved: (msg: string) => void;
}) {
  const [username, setUsername] = useState(edit?.username ?? "");
  const [password, setPassword] = useState("");
  const [groupIds, setGroupIds] = useState<number[]>(edit ? edit.groups.map((g) => g.id) : []);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const toggleGroup = (id: number) =>
    setGroupIds((cur) => (cur.includes(id) ? cur.filter((x) => x !== id) : [...cur, id]));

  async function save() {
    setBusy(true);
    setError("");
    try {
      if (edit) {
        await admin.updateUser(token, edit.id, {
          password: password || undefined,
          group_ids: groupIds,
        });
        onSaved("已更新 " + edit.username);
      } else {
        await admin.createUser(token, username.trim(), password, groupIds);
        onSaved("已创建 " + username.trim());
      }
    } catch (e) {
      setError((e as Error).message);
      setBusy(false);
    }
  }

  return (
    <Modal
      title={edit ? "编辑用户" : "新建用户"}
      subtitle={edit ? edit.username : "创建一个登录账号并指定分组"}
      onClose={onClose}
      footer={
        <>
          <button
            onClick={onClose}
            className="rounded-xl border border-slate-200 px-4 py-2 text-sm font-semibold text-slate-600 transition hover:bg-slate-50"
          >
            取消
          </button>
          <button
            onClick={save}
            disabled={busy}
            className="rounded-xl bg-indigo-500 px-4 py-2 text-sm font-semibold text-white transition hover:bg-indigo-600 disabled:opacity-60"
          >
            {busy ? "保存中…" : "保存"}
          </button>
        </>
      }
    >
      <div className="space-y-4">
        {!edit && (
          <Field label="用户名">
            <input
              autoFocus
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              placeholder="字母 / 数字 / _ / -"
              className={inputCls}
            />
          </Field>
        )}
        <Field label={edit ? "重置密码（留空则不变）" : "密码（至少 6 位）"}>
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder={edit ? "••••••" : "至少 6 位"}
            className={inputCls}
          />
        </Field>
        <Field label="分组（可多选）">
          {groups.length === 0 ? (
            <p className="rounded-lg bg-amber-50 px-3 py-2 text-xs text-amber-600">
              还没有分组，请先在「分组」中创建。
            </p>
          ) : (
            <div className="flex flex-wrap gap-2">
              {groups.map((g) => {
                const on = groupIds.includes(g.id);
                return (
                  <button
                    key={g.id}
                    type="button"
                    onClick={() => toggleGroup(g.id)}
                    className={`rounded-full border px-3 py-1.5 text-xs font-medium transition ${
                      on
                        ? "border-indigo-300 bg-indigo-50 text-indigo-600"
                        : "border-slate-200 bg-white text-slate-500 hover:bg-slate-50"
                    }`}
                  >
                    {g.name}
                  </button>
                );
              })}
            </div>
          )}
        </Field>
        <ModalError error={error} />
      </div>
    </Modal>
  );
}

// ---------------------------------------------------------------------------
// 分组 CRUD
// ---------------------------------------------------------------------------

function GroupsTable({ token, notify }: { token: string; notify: (m: string) => void }) {
  const { data, loading, error, reload } = useAdminData<AdminGroup[]>(
    () => admin.groups(token),
    [token],
  );
  const [modal, setModal] = useState<{ edit: AdminGroup | null } | null>(null);
  if (loading || error) return <StateLine loading={loading} error={error} />;
  const rows = data!;

  async function del(g: AdminGroup) {
    if (!confirm(`删除分组「${g.name}」？\n该分组下 ${g.users} 个成员将被移出分组（不会删除用户）。`))
      return;
    try {
      await admin.deleteGroup(token, g.id);
      notify("已删除");
      reload();
    } catch (e) {
      notify((e as Error).message);
    }
  }

  return (
    <div className="space-y-4">
      <Toolbar
        title="分组"
        hint="分组用于控制市场技能 / MCP 对哪些用户可见。"
        action={
          <PrimaryButton onClick={() => setModal({ edit: null })}>
            <PlusIcon width={16} height={16} /> 新建分组
          </PrimaryButton>
        }
      />
      {rows.length === 0 ? (
        <Panel>还没有分组。点击「新建分组」创建第一个。</Panel>
      ) : (
        <Table head={["分组名", "描述", "成员数", "创建于", "操作"]}>
          {rows.map((g) => (
            <tr key={g.id} className="text-slate-700">
              <td className="px-5 py-3 font-semibold text-slate-800">{g.name}</td>
              <td className="px-5 py-3 text-slate-500">{g.description || "—"}</td>
              <td className="px-5 py-3 text-slate-500">{g.users}</td>
              <td className="px-5 py-3 text-xs text-slate-400">{g.created_at}</td>
              <td className="px-5 py-3">
                <RowActions>
                  <MiniButton onClick={() => setModal({ edit: g })}>编辑</MiniButton>
                  <MiniButton tone="rose" onClick={() => del(g)}>
                    <TrashIcon width={13} height={13} />
                  </MiniButton>
                </RowActions>
              </td>
            </tr>
          ))}
        </Table>
      )}
      {modal && (
        <GroupModal
          token={token}
          edit={modal.edit}
          onClose={() => setModal(null)}
          onSaved={(msg) => {
            setModal(null);
            notify(msg);
            reload();
          }}
        />
      )}
    </div>
  );
}

function GroupModal({
  token,
  edit,
  onClose,
  onSaved,
}: {
  token: string;
  edit: AdminGroup | null;
  onClose: () => void;
  onSaved: (msg: string) => void;
}) {
  const [name, setName] = useState(edit?.name ?? "");
  const [description, setDescription] = useState(edit?.description ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  async function save() {
    setBusy(true);
    setError("");
    try {
      if (edit) {
        await admin.updateGroup(token, edit.id, { name: name.trim(), description });
        onSaved("已更新 " + name.trim());
      } else {
        await admin.createGroup(token, name.trim(), description);
        onSaved("已创建 " + name.trim());
      }
    } catch (e) {
      setError((e as Error).message);
      setBusy(false);
    }
  }

  return (
    <Modal
      title={edit ? "编辑分组" : "新建分组"}
      onClose={onClose}
      footer={
        <>
          <button
            onClick={onClose}
            className="rounded-xl border border-slate-200 px-4 py-2 text-sm font-semibold text-slate-600 transition hover:bg-slate-50"
          >
            取消
          </button>
          <button
            onClick={save}
            disabled={busy}
            className="rounded-xl bg-indigo-500 px-4 py-2 text-sm font-semibold text-white transition hover:bg-indigo-600 disabled:opacity-60"
          >
            {busy ? "保存中…" : "保存"}
          </button>
        </>
      }
    >
      <div className="space-y-4">
        <Field label="分组名">
          <input
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="如：内部团队 / 合作伙伴"
            className={inputCls}
          />
        </Field>
        <Field label="描述（可选）">
          <input
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="用途说明"
            className={inputCls}
          />
        </Field>
        <ModalError error={error} />
      </div>
    </Modal>
  );
}

// ---------------------------------------------------------------------------
// 标签（labels）CRUD
// ---------------------------------------------------------------------------

function LabelsTable({ token, notify }: { token: string; notify: (m: string) => void }) {
  const { data, loading, error, reload } = useAdminData<AdminLabel[]>(
    () => admin.labels(token),
    [token],
  );
  const [modal, setModal] = useState<{ edit: AdminLabel | null } | null>(null);
  if (loading || error) return <StateLine loading={loading} error={error} />;
  const rows = data!;

  async function del(l: AdminLabel) {
    if (
      !confirm(
        `删除标签「${l.name}」？\n将从 ${l.skills} 个技能、${l.mcps} 个 MCP 上移除该标签（不删除资源）。`,
      )
    )
      return;
    try {
      await admin.deleteLabel(token, l.id);
      notify("已删除");
      reload();
    } catch (e) {
      notify((e as Error).message);
    }
  }

  return (
    <div className="space-y-4">
      <Toolbar
        title="标签"
        hint="受管标签用于标注与筛选市场资源（默认内置「官方」「社区」）。在资源「配置」中分配。"
        action={
          <PrimaryButton onClick={() => setModal({ edit: null })}>
            <PlusIcon width={16} height={16} /> 新建标签
          </PrimaryButton>
        }
      />
      {rows.length === 0 ? (
        <Panel>还没有标签。点击「新建标签」创建。</Panel>
      ) : (
        <Table head={["标签", "技能", "MCP", "创建于", "操作"]}>
          {rows.map((l) => (
            <tr key={l.id} className="text-slate-700">
              <td className="px-5 py-3 font-semibold text-slate-800">{l.name}</td>
              <td className="px-5 py-3 text-slate-500">{l.skills}</td>
              <td className="px-5 py-3 text-slate-500">{l.mcps}</td>
              <td className="px-5 py-3 text-xs text-slate-400">{l.created_at || "—"}</td>
              <td className="px-5 py-3">
                <RowActions>
                  <MiniButton onClick={() => setModal({ edit: l })}>编辑</MiniButton>
                  <MiniButton tone="rose" onClick={() => del(l)}>
                    <TrashIcon width={13} height={13} />
                  </MiniButton>
                </RowActions>
              </td>
            </tr>
          ))}
        </Table>
      )}
      {modal && (
        <LabelModal
          token={token}
          edit={modal.edit}
          onClose={() => setModal(null)}
          onSaved={(msg) => {
            setModal(null);
            notify(msg);
            reload();
          }}
        />
      )}
    </div>
  );
}

function LabelModal({
  token,
  edit,
  onClose,
  onSaved,
}: {
  token: string;
  edit: AdminLabel | null;
  onClose: () => void;
  onSaved: (msg: string) => void;
}) {
  const [name, setName] = useState(edit?.name ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  async function save() {
    setBusy(true);
    setError("");
    try {
      if (edit) {
        await admin.updateLabel(token, edit.id, name.trim());
        onSaved("已更新 " + name.trim());
      } else {
        await admin.createLabel(token, name.trim());
        onSaved("已创建 " + name.trim());
      }
    } catch (e) {
      setError((e as Error).message);
      setBusy(false);
    }
  }

  return (
    <Modal
      title={edit ? "编辑标签" : "新建标签"}
      onClose={onClose}
      footer={
        <>
          <button
            onClick={onClose}
            className="rounded-xl border border-slate-200 px-4 py-2 text-sm font-semibold text-slate-600 transition hover:bg-slate-50"
          >
            取消
          </button>
          <button
            onClick={save}
            disabled={busy}
            className="rounded-xl bg-indigo-500 px-4 py-2 text-sm font-semibold text-white transition hover:bg-indigo-600 disabled:opacity-60"
          >
            {busy ? "保存中…" : "保存"}
          </button>
        </>
      }
    >
      <div className="space-y-4">
        <Field label="标签名">
          <input
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="如：官方 / 社区 / 精选"
            className={inputCls}
          />
        </Field>
        <ModalError error={error} />
      </div>
    </Modal>
  );
}

/** 时间窗口选项（小时；0 表示不限）。 */
const CALL_WINDOWS = [
  { v: 24, label: "近 24 小时" },
  { v: 168, label: "近 7 天" },
  { v: 720, label: "近 30 天" },
  { v: 0, label: "全部" },
] as const;

const CALLS_PAGE_SIZE = 20;

const selectCls = `${inputCls} appearance-none bg-[length:18px] bg-[right_0.6rem_center] bg-no-repeat pr-9`;
const caretBg = {
  backgroundImage:
    "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='18' height='18' viewBox='0 0 24 24' fill='none' stroke='%2394a3b8' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='m6 9 6 6 6-6'/%3E%3C/svg%3E\")",
};

/** 绿/红状态药丸，对齐参考稿（ok / err）。 */
function StatusPill({ ok }: { ok: boolean }) {
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-semibold ${
        ok ? "bg-emerald-50 text-emerald-600" : "bg-rose-50 text-rose-500"
      }`}
    >
      <span className={`size-1.5 rounded-full ${ok ? "bg-emerald-500" : "bg-rose-500"}`} />
      {ok ? "ok" : "err"}
    </span>
  );
}

/** 极简开关（仅错误）。 */
function Toggle({ on, onChange }: { on: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      onClick={() => onChange(!on)}
      className={`relative inline-flex h-6 w-11 flex-none items-center rounded-full transition ${
        on ? "bg-indigo-500" : "bg-slate-200"
      }`}
    >
      <span
        className={`inline-block size-5 transform rounded-full bg-white shadow transition ${
          on ? "translate-x-5" : "translate-x-0.5"
        }`}
      />
    </button>
  );
}

function CallsTable({ token }: { token: string }) {
  // 已应用的过滤条件（点「查询」才生效）与草稿态分离，避免边输边查。
  const [applied, setApplied] = useState<CallsQuery>({ window: 24 });
  const [service, setService] = useState("");
  const [tool, setTool] = useState("");
  const [caller, setCaller] = useState("");
  const [window, setWindow] = useState(24);
  const [errorsOnly, setErrorsOnly] = useState(false);
  const [page, setPage] = useState(0);

  const [data, setData] = useState<CallsResp | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  useEffect(() => {
    let alive = true;
    setLoading(true);
    setError("");
    admin
      .calls(token, { ...applied, limit: CALLS_PAGE_SIZE, offset: page * CALLS_PAGE_SIZE })
      .then((d) => alive && setData(d))
      .catch((e) => alive && setError((e as Error).message))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [token, applied, page]);

  function query() {
    setApplied({
      service: service || undefined,
      tool: tool || undefined,
      caller: caller.trim() || undefined,
      window: window || undefined,
      errors_only: errorsOnly || undefined,
    });
    setPage(0);
  }
  function reset() {
    setService("");
    setTool("");
    setCaller("");
    setWindow(24);
    setErrorsOnly(false);
    setApplied({ window: 24 });
    setPage(0);
  }

  const services = data?.services ?? [];
  const tools = data?.tools ?? [];
  const total = data?.total ?? 0;
  const rows = data?.rows ?? [];
  const lastPage = Math.max(0, Math.ceil(total / CALLS_PAGE_SIZE) - 1);

  return (
    <div className="space-y-5">
      <Panel>
        <h2 className="mb-4 font-bold text-slate-800">调用日志</h2>
        <div className="grid grid-cols-2 items-end gap-4 lg:grid-cols-5">
          <label className="block">
            <span className={adminLabelCls}>服务</span>
            <select
              className={selectCls}
              style={caretBg}
              value={service}
              onChange={(e) => setService(e.target.value)}
            >
              <option value="">全部服务</option>
              {services.map((s) => (
                <option key={s} value={s}>
                  {s}
                </option>
              ))}
            </select>
          </label>
          <label className="block">
            <span className={adminLabelCls}>工具</span>
            <select
              className={selectCls}
              style={caretBg}
              value={tool}
              onChange={(e) => setTool(e.target.value)}
            >
              <option value="">全部工具</option>
              {tools.map((t) => (
                <option key={t} value={t}>
                  {t}
                </option>
              ))}
            </select>
          </label>
          <label className="block">
            <span className={adminLabelCls}>用户</span>
            <input
              className={inputCls}
              placeholder="任意"
              value={caller}
              onChange={(e) => setCaller(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && query()}
            />
          </label>
          <label className="block">
            <span className={adminLabelCls}>时间窗口</span>
            <select
              className={selectCls}
              style={caretBg}
              value={window}
              onChange={(e) => setWindow(Number(e.target.value))}
            >
              {CALL_WINDOWS.map((w) => (
                <option key={w.v} value={w.v}>
                  {w.label}
                </option>
              ))}
            </select>
          </label>
          <div>
            <span className={adminLabelCls}>仅错误</span>
            <div className="flex h-[42px] items-center">
              <Toggle on={errorsOnly} onChange={setErrorsOnly} />
            </div>
          </div>
        </div>
        <div className="mt-4 flex gap-2.5">
          <button
            onClick={query}
            className="rounded-xl bg-indigo-500 px-5 py-2 text-sm font-semibold text-white transition hover:bg-indigo-600"
          >
            查询
          </button>
          <button
            onClick={reset}
            className="rounded-xl border border-slate-200 px-5 py-2 text-sm font-semibold text-slate-600 transition hover:bg-slate-50"
          >
            重置
          </button>
        </div>
      </Panel>

      <Panel className="p-0">
        <div className="flex items-center justify-between gap-4 px-6 py-4">
          <div className="text-sm text-slate-500">
            匹配 <span className="font-semibold text-slate-800">{total}</span> 条
          </div>
          <div className="flex gap-2">
            <button
              onClick={() => setPage((p) => Math.max(0, p - 1))}
              disabled={page <= 0}
              className="rounded-lg border border-slate-200 px-3 py-1.5 text-sm font-medium text-slate-600 transition hover:bg-slate-50 disabled:cursor-not-allowed disabled:opacity-40"
            >
              上一页
            </button>
            <button
              onClick={() => setPage((p) => Math.min(lastPage, p + 1))}
              disabled={page >= lastPage}
              className="rounded-lg border border-slate-200 px-3 py-1.5 text-sm font-medium text-slate-600 transition hover:bg-slate-50 disabled:cursor-not-allowed disabled:opacity-40"
            >
              下一页
            </button>
          </div>
        </div>

        {loading ? (
          <StateLine loading error="" />
        ) : error ? (
          <StateLine loading={false} error={error} />
        ) : rows.length === 0 ? (
          <div className="px-6 py-16 text-center text-slate-400">暂无匹配的调用日志。</div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-left text-sm">
              <thead>
                <tr className="border-t border-slate-100 text-xs uppercase tracking-wide text-slate-400">
                  {["时间", "状态", "用户", "服务 / 工具", "耗时", "结果摘要"].map((h) => (
                    <th key={h} className="px-6 py-3 font-medium">
                      {h}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-100">
                {rows.map((c, i) => (
                  <CallRow key={i} c={c} />
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Panel>
    </div>
  );
}

function CallRow({ c }: { c: CallLog }) {
  const summary = c.ok ? c.result : c.error;
  return (
    <tr className="text-slate-700 transition hover:bg-slate-50/60">
      <td className="whitespace-nowrap px-6 py-3.5 text-sm text-slate-500">{c.created_at}</td>
      <td className="px-6 py-3.5">
        <StatusPill ok={c.ok} />
      </td>
      <td className="px-6 py-3.5">
        <div className="font-semibold text-slate-800">{c.caller || "—"}</div>
        {c.caller_id != null && <div className="text-xs text-slate-400">id: {c.caller_id}</div>}
      </td>
      <td className="px-6 py-3.5">
        <span className="rounded-md bg-slate-100 px-2 py-1 font-mono text-xs font-medium text-slate-600">
          {c.mcp_name}/{c.tool}
        </span>
        <div className="mt-0.5 text-xs text-slate-400">@{c.owner}</div>
      </td>
      <td className="whitespace-nowrap px-6 py-3.5 text-slate-500">{c.ms} ms</td>
      <td className="px-6 py-3.5">
        <div
          className={`max-w-md truncate font-mono text-xs ${c.ok ? "text-slate-500" : "text-rose-500"}`}
          title={summary}
        >
          {summary || "—"}
        </div>
      </td>
    </tr>
  );
}

// ---------------------------------------------------------------------------
// 数据迁移：导入 / 导出
// ---------------------------------------------------------------------------

function Migrate({ token, notify }: { token: string; notify: (m: string) => void }) {
  const [busy, setBusy] = useState(false);
  const [summary, setSummary] = useState<ImportSummary | null>(null);
  const [error, setError] = useState("");
  const fileRef = useRef<HTMLInputElement>(null);

  async function doExport() {
    setBusy(true);
    setError("");
    try {
      await admin.exportPack(token);
      notify("已导出资源包");
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function doImport(file: File) {
    if (!confirm(`确认导入「${file.name}」？\n将以合并(upsert)方式写入当前实例，按自然键覆盖同名资源。`)) {
      return;
    }
    setBusy(true);
    setError("");
    setSummary(null);
    try {
      const res = await admin.importPack(token, file);
      setSummary(res);
      notify("导入完成");
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
      if (fileRef.current) fileRef.current.value = "";
    }
  }

  return (
    <div className="space-y-6">
      <Panel>
        <h2 className="font-bold text-slate-800">导出全量资源包</h2>
        <p className="mt-2 text-sm text-slate-500">
          打包全部用户、技能、MCP、加密凭据、调用日志与压缩体（按 sha256 内容寻址）为单个{" "}
          <code className="rounded bg-slate-100 px-1 py-0.5 font-mono text-xs">.tskpack</code>（tar + zstd）。
        </p>
        <button
          onClick={doExport}
          disabled={busy}
          className="mt-4 flex items-center gap-2 rounded-xl bg-indigo-500 px-5 py-2.5 text-sm font-semibold text-white transition hover:bg-indigo-600 disabled:opacity-60"
        >
          <DownloadIcon width={16} height={16} /> {busy ? "处理中…" : "导出 .tskpack"}
        </button>
      </Panel>

      <Panel>
        <h2 className="font-bold text-slate-800">导入资源包</h2>
        <p className="mt-2 text-sm text-slate-500">
          上传 <code className="rounded bg-slate-100 px-1 py-0.5 font-mono text-xs">.tskpack</code>{" "}
          导入到当前实例。采用<strong className="text-slate-600"> 合并(upsert) </strong>
          语义：按用户名 / (owner,name) / (owner,key) 覆盖更新，不删除已有数据。
        </p>
        <div className="mt-3 rounded-xl border border-amber-200 bg-amber-50 px-4 py-3 text-xs text-amber-700">
          注意：加密凭据以「nonce + 密文」原样迁移，需目标实例共用同一{" "}
          <code className="font-mono">master.key</code>（或 <code className="font-mono">TRISKELION_MASTER_KEY</code>）方可解密。
        </div>
        <input
          ref={fileRef}
          type="file"
          accept=".tskpack,application/zstd,application/octet-stream"
          onChange={(e) => {
            const f = e.target.files?.[0];
            if (f) doImport(f);
          }}
          className="mt-4 block w-full text-sm text-slate-500 file:mr-4 file:cursor-pointer file:rounded-xl file:border-0 file:bg-indigo-50 file:px-5 file:py-2.5 file:text-sm file:font-semibold file:text-indigo-600 hover:file:bg-indigo-100"
        />
        {error && <div className="mt-3 text-sm text-rose-500">导入失败：{error}</div>}
        {summary && (
          <div className="mt-4 rounded-xl border border-emerald-200 bg-emerald-50 px-4 py-3 text-sm text-emerald-700">
            <div className="font-semibold">导入完成</div>
            <div className="mt-1 text-emerald-600">
              用户 {summary.users} · MCP {summary.mcps} · 技能 {summary.skills} · 凭据 {summary.secrets} ·
              调用日志 {summary.calls} · 新增压缩体 {summary.blobs}
            </div>
            {summary.skipped.length > 0 && (
              <ul className="mt-2 list-inside list-disc text-xs text-amber-600">
                {summary.skipped.map((s, i) => (
                  <li key={i}>{s}</li>
                ))}
              </ul>
            )}
          </div>
        )}
      </Panel>
    </div>
  );
}
