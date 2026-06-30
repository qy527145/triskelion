import { useCallback, useEffect, useRef, useState } from "react";
import { admin, ApiError } from "../lib/api";
import { humanSize, parseGroupVisibility, SKILL_CATEGORIES } from "../lib/types";
import type {
  AdminGroup,
  AdminLabel,
  AdminMcp,
  AdminSkill,
  AdminStats,
  AdminUser,
  CallLog,
  GroupVisibility,
  ImportSummary,
  McpManifest,
  Protocol,
  Runtime,
  SkillCategory,
} from "../lib/types";
import Modal from "./Modal";
import Brand from "./Brand";
import { DownloadIcon, LogoutIcon, PlusIcon, Spinner, TrashIcon } from "./icons";

const TOKEN_KEY = "tsk_admin_token";

const TABS = [
  { id: "overview", label: "概览" },
  { id: "skills", label: "技能" },
  { id: "mcps", label: "MCP 服务" },
  { id: "users", label: "用户" },
  { id: "groups", label: "分组" },
  { id: "labels", label: "标签" },
  { id: "calls", label: "调用日志" },
  { id: "migrate", label: "数据迁移" },
] as const;
type AdminTab = (typeof TABS)[number]["id"];

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
  const [tab, setTab] = useState<AdminTab>("overview");
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
          <nav className="flex w-44 flex-none flex-col gap-1 self-start rounded-2xl border border-slate-200 bg-white p-2 shadow-sm">
            {TABS.map((t) => (
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
          </nav>

          <div className="min-w-0 flex-1">
            {tab === "overview" && <Overview token={token} />}
            {tab === "skills" && <SkillsTable token={token} notify={notify} />}
            {tab === "mcps" && <McpsTable token={token} notify={notify} />}
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

function Table({ head, children }: { head: string[]; children: React.ReactNode }) {
  return (
    <Panel className="overflow-x-auto p-0">
      <table className="w-full text-left text-sm">
        <thead>
          <tr className="border-b border-slate-200 text-xs uppercase tracking-wide text-slate-400">
            {head.map((h) => (
              <th key={h} className="px-5 py-3 font-medium">
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



function SkillsTable({ token, notify }: { token: string; notify: (m: string) => void }) {
  const { data, loading, error, reload } = useAdminData<AdminSkill[]>(
    () => admin.skills(token),
    [token],
  );
  const groups = useGroups(token);
  const labels = useLabels(token);
  const [edit, setEdit] = useState<AdminSkill | null>(null);
  const [content, setContent] = useState<AdminSkill | null>(null);
  if (loading || error) return <StateLine loading={loading} error={error} />;
  const rows = data!;

  async function del(s: AdminSkill) {
    if (!confirm(`删除技能「${s.owner}/${s.name}」？此操作不可恢复。`)) return;
    try {
      await admin.deleteSkill(token, s.owner, s.name);
      notify("已删除");
      reload();
    } catch (e) {
      notify((e as Error).message);
    }
  }

  return (
    <div>
      {rows.length === 0 ? (
        <Panel>暂无技能。</Panel>
      ) : (
        <Table head={["技能", "分类", "可见性", "可见分组", "标签", "压缩体", "更新于", "操作"]}>
          {rows.map((s) => (
            <tr key={s.owner + "/" + s.name} className="text-slate-700">
              <td className="px-5 py-3">
                <div className="font-semibold text-slate-800">{s.name}</div>
                <div className="text-xs text-slate-400">@{s.owner}</div>
              </td>
              <td className="px-5 py-3 text-slate-500">{s.category}</td>
              <td className="px-5 py-3">
                <Badge kind={s.visibility === "public" ? "public" : "private"}>{s.visibility}</Badge>
              </td>
              <td className="px-5 py-3 text-xs text-slate-500">
                {s.visibility === "public" ? groupVisLabel(s.group_visibility, groups) : "—"}
              </td>
              <td className="px-5 py-3">
                <LabelBadges labels={s.labels} />
              </td>
              <td className="px-5 py-3 text-slate-500">
                {s.has_archive ? `📦 ${humanSize(s.archive_size)}` : "纯文本"}
              </td>
              <td className="px-5 py-3 text-xs text-slate-400">{s.updated_at}</td>
              <td className="px-5 py-3">
                <RowActions>
                  <MiniButton onClick={() => setContent(s)}>编辑</MiniButton>
                  <MiniButton onClick={() => setEdit(s)}>配置</MiniButton>
                  <MiniButton tone="rose" onClick={() => del(s)}>
                    <TrashIcon width={13} height={13} />
                  </MiniButton>
                </RowActions>
              </td>
            </tr>
          ))}
        </Table>
      )}
      {edit && (
        <VisibilityModal
          kind="skill"
          token={token}
          owner={edit.owner}
          name={edit.name}
          visibility={edit.visibility}
          groupVisibilityRaw={edit.group_visibility}
          groups={groups}
          labels={labels}
          currentLabelIds={edit.labels.map((l) => l.id)}
          onClose={() => setEdit(null)}
          onSaved={() => {
            setEdit(null);
            notify("已更新");
            reload();
          }}
        />
      )}
      {content && (
        <AdminSkillEditModal
          token={token}
          skill={content}
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

function McpsTable({ token, notify }: { token: string; notify: (m: string) => void }) {
  const { data, loading, error, reload } = useAdminData<AdminMcp[]>(
    () => admin.mcps(token),
    [token],
  );
  const groups = useGroups(token);
  const labels = useLabels(token);
  const [edit, setEdit] = useState<AdminMcp | null>(null);
  const [content, setContent] = useState<AdminMcp | null>(null);
  if (loading || error) return <StateLine loading={loading} error={error} />;
  const rows = data!;

  async function del(m: AdminMcp) {
    if (!confirm(`删除 MCP「${m.owner}/${m.name}」？此操作不可恢复。`)) return;
    try {
      await admin.deleteMcp(token, m.owner, m.name);
      notify("已删除");
      reload();
    } catch (e) {
      notify((e as Error).message);
    }
  }

  return (
    <div>
      {rows.length === 0 ? (
        <Panel>暂无 MCP。</Panel>
      ) : (
        <Table head={["MCP", "可见性", "可见分组", "标签", "运行时", "协议", "更新于", "操作"]}>
          {rows.map((m) => (
            <tr key={m.owner + "/" + m.name} className="text-slate-700">
              <td className="px-5 py-3">
                <div className="font-semibold text-slate-800">{m.name}</div>
                <div className="text-xs text-slate-400">@{m.owner}</div>
              </td>
              <td className="px-5 py-3">
                <Badge kind={m.visibility === "public" ? "public" : "private"}>{m.visibility}</Badge>
              </td>
              <td className="px-5 py-3 text-xs text-slate-500">
                {m.visibility === "public" ? groupVisLabel(m.group_visibility, groups) : "—"}
              </td>
              <td className="px-5 py-3">
                <LabelBadges labels={m.labels} />
              </td>
              <td className="px-5 py-3 text-slate-500">{m.runtime}</td>
              <td className="px-5 py-3 text-slate-500">{m.protocol}</td>
              <td className="px-5 py-3 text-xs text-slate-400">{m.updated_at}</td>
              <td className="px-5 py-3">
                <RowActions>
                  <MiniButton onClick={() => setContent(m)}>编辑</MiniButton>
                  <MiniButton onClick={() => setEdit(m)}>配置</MiniButton>
                  <MiniButton tone="rose" onClick={() => del(m)}>
                    <TrashIcon width={13} height={13} />
                  </MiniButton>
                </RowActions>
              </td>
            </tr>
          ))}
        </Table>
      )}
      {edit && (
        <VisibilityModal
          kind="mcp"
          token={token}
          owner={edit.owner}
          name={edit.name}
          visibility={edit.visibility}
          groupVisibilityRaw={edit.group_visibility}
          groups={groups}
          labels={labels}
          currentLabelIds={edit.labels.map((l) => l.id)}
          onClose={() => setEdit(null)}
          onSaved={() => {
            setEdit(null);
            notify("已更新");
            reload();
          }}
        />
      )}
      {content && (
        <AdminMcpEditModal
          token={token}
          mcp={content}
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
}: {
  token: string;
  skill: AdminSkill;
  onClose: () => void;
  onSaved: () => void;
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

  async function save() {
    if (!skillMd.trim()) {
      setError("SKILL.md 不能为空");
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
          <label className={adminLabelCls}>SKILL.md</label>
          <textarea
            className={`${inputCls} min-h-[180px] resize-y font-mono text-xs leading-relaxed`}
            value={skillMd}
            onChange={(e) => setSkillMd(e.target.value)}
          />
        </div>
        {skill.has_archive && (
          <p className="rounded-lg bg-amber-50 px-3 py-2 text-xs text-amber-600">
            该技能含压缩体：此处仅改写元数据与展示用的 SKILL.md，压缩包内文件不变。
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
        hint="创建用户、调整分组归属、重置密码或删除。"
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
    </div>
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

function CallsTable({ token }: { token: string }) {
  const { data, loading, error } = useAdminData<CallLog[]>(() => admin.calls(token), [token]);
  if (loading || error) return <StateLine loading={loading} error={error} />;
  const rows = data!;
  if (rows.length === 0) return <Panel>暂无调用日志。</Panel>;
  return (
    <Table head={["状态", "工具", "调用者", "耗时", "时间", "错误"]}>
      {rows.map((c, i) => (
        <tr key={i} className="text-slate-700">
          <td className="px-5 py-3">
            <Badge kind={c.ok ? "ok" : "err"}>{c.ok ? "成功" : "失败"}</Badge>
          </td>
          <td className="px-5 py-3 font-mono text-xs text-slate-600">
            {c.mcp_name}/{c.tool}
            <div className="text-slate-400">@{c.owner}</div>
          </td>
          <td className="px-5 py-3 text-slate-500">{c.caller || "—"}</td>
          <td className="px-5 py-3 text-slate-500">{c.ms}ms</td>
          <td className="px-5 py-3 text-xs text-slate-400">{c.created_at}</td>
          <td className="px-5 py-3 max-w-xs truncate font-mono text-xs text-rose-500">{c.error}</td>
        </tr>
      ))}
    </Table>
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
