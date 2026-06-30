import { useCallback, useEffect, useRef, useState } from "react";
import { admin, ApiError } from "../lib/api";
import { humanSize } from "../lib/types";
import type {
  AdminMcp,
  AdminSkill,
  AdminStats,
  AdminUser,
  CallLog,
  ImportSummary,
} from "../lib/types";
import { DownloadIcon, LogoutIcon } from "./icons";

const TOKEN_KEY = "tsk_admin_token";

const TABS = [
  { id: "overview", label: "概览" },
  { id: "skills", label: "技能" },
  { id: "mcps", label: "MCP 服务" },
  { id: "users", label: "用户" },
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
    <div className="grid min-h-screen place-items-center bg-slate-50 px-6">
      <div className="w-full max-w-sm rounded-2xl border border-slate-200 bg-white p-8 shadow-sm">
        <div className="mb-6 flex items-center gap-2.5 text-lg font-bold text-slate-800">
          <span className="grid size-9 place-items-center rounded-[10px] bg-gradient-to-br from-indigo-500 to-violet-500 font-extrabold text-white">
            T
          </span>
          管理后台
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
    <div className="min-h-screen bg-slate-50">
      <header className="sticky top-0 z-40 flex items-center gap-3 border-b border-slate-200 bg-white/90 px-6 py-3.5 backdrop-blur">
        <div className="flex items-center gap-2.5 text-lg font-bold text-slate-800">
          <span className="grid size-9 place-items-center rounded-[10px] bg-gradient-to-br from-indigo-500 to-violet-500 font-extrabold text-white">
            T
          </span>
          triskelion · 管理后台
        </div>
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
            {tab === "skills" && <SkillsTable token={token} />}
            {tab === "mcps" && <McpsTable token={token} />}
            {tab === "users" && <UsersTable token={token} />}
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
  if (loading) return <div className="py-16 text-center text-slate-400">加载中…</div>;
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
          <div key={c.label} className="rounded-2xl border border-slate-200 bg-white p-5 shadow-sm">
            <div className="text-sm text-slate-400">{c.label}</div>
            <div className="mt-2 text-3xl font-bold text-slate-800">{c.value}</div>
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

function SkillsTable({ token }: { token: string }) {
  const { data, loading, error } = useAdminData<AdminSkill[]>(() => admin.skills(token), [token]);
  if (loading || error) return <StateLine loading={loading} error={error} />;
  const rows = data!;
  if (rows.length === 0) return <Panel>暂无技能。</Panel>;
  return (
    <Table head={["技能", "分类", "可见性", "版本", "压缩体", "更新于"]}>
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
          <td className="px-5 py-3 text-slate-500">v{s.version}</td>
          <td className="px-5 py-3 text-slate-500">
            {s.has_archive ? `📦 ${humanSize(s.archive_size)}` : "纯文本"}
          </td>
          <td className="px-5 py-3 text-xs text-slate-400">{s.updated_at}</td>
        </tr>
      ))}
    </Table>
  );
}

function McpsTable({ token }: { token: string }) {
  const { data, loading, error } = useAdminData<AdminMcp[]>(() => admin.mcps(token), [token]);
  if (loading || error) return <StateLine loading={loading} error={error} />;
  const rows = data!;
  if (rows.length === 0) return <Panel>暂无 MCP。</Panel>;
  return (
    <Table head={["MCP", "可见性", "版本", "运行时", "协议", "更新于"]}>
      {rows.map((m) => (
        <tr key={m.owner + "/" + m.name} className="text-slate-700">
          <td className="px-5 py-3">
            <div className="font-semibold text-slate-800">{m.name}</div>
            <div className="text-xs text-slate-400">@{m.owner}</div>
          </td>
          <td className="px-5 py-3">
            <Badge kind={m.visibility === "public" ? "public" : "private"}>{m.visibility}</Badge>
          </td>
          <td className="px-5 py-3 text-slate-500">v{m.version}</td>
          <td className="px-5 py-3 text-slate-500">{m.runtime}</td>
          <td className="px-5 py-3 text-slate-500">{m.protocol}</td>
          <td className="px-5 py-3 text-xs text-slate-400">{m.updated_at}</td>
        </tr>
      ))}
    </Table>
  );
}

function UsersTable({ token }: { token: string }) {
  const { data, loading, error } = useAdminData<AdminUser[]>(() => admin.users(token), [token]);
  if (loading || error) return <StateLine loading={loading} error={error} />;
  const rows = data!;
  return (
    <Table head={["用户名", "技能", "MCP", "凭据", "注册于"]}>
      {rows.map((u) => (
        <tr key={u.username} className="text-slate-700">
          <td className="px-5 py-3 font-semibold text-slate-800">{u.username}</td>
          <td className="px-5 py-3 text-slate-500">{u.skills}</td>
          <td className="px-5 py-3 text-slate-500">{u.mcps}</td>
          <td className="px-5 py-3 text-slate-500">{u.secrets}</td>
          <td className="px-5 py-3 text-xs text-slate-400">{u.created_at}</td>
        </tr>
      ))}
    </Table>
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
