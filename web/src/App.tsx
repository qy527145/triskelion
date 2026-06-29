import { useCallback, useEffect, useState } from "react";
import Header, { type Tab } from "./components/Header";
import McpCard from "./components/McpCard";
import LoginModal from "./components/LoginModal";
import CreateMcpModal from "./components/CreateMcpModal";
import DetailModal from "./components/DetailModal";
import SecretModal from "./components/SecretModal";
import { SearchIcon, PlusIcon, KeyIcon, TrashIcon } from "./components/icons";
import { api, clearAuth, getUser } from "./lib/api";
import type { McpInfo, SecretInfo } from "./lib/types";

export default function App() {
  const [user, setUser] = useState<string | null>(getUser());
  const [tab, setTab] = useState<Tab>("market");

  const [items, setItems] = useState<McpInfo[]>([]);
  const [secrets, setSecrets] = useState<SecretInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [query, setQuery] = useState("");
  const [search, setSearch] = useState("");

  const [showLogin, setShowLogin] = useState(false);
  const [mcpModal, setMcpModal] = useState<{ edit: McpInfo | null } | null>(null);
  const [detail, setDetail] = useState<McpInfo | null>(null);
  const [secretEdit, setSecretEdit] = useState<{ key: string | null } | null>(null);
  const [toast, setToast] = useState("");

  const notify = useCallback((msg: string) => {
    setToast(msg);
    setTimeout(() => setToast(""), 1900);
  }, []);

  const load = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      if (tab === "secrets") {
        setSecrets(await api.listSecrets());
      } else if (tab === "mine") {
        setItems(await api.listMine());
      } else {
        setItems(await api.explore(search));
      }
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  }, [tab, search]);

  useEffect(() => {
    load();
  }, [load]);

  function switchTab(t: Tab) {
    if ((t === "mine" || t === "secrets") && !user) {
      setShowLogin(true);
      return;
    }
    setTab(t);
  }

  function onAuthed(username: string) {
    setUser(username);
    setShowLogin(false);
    notify("欢迎，" + username);
  }
  function logout() {
    clearAuth();
    setUser(null);
    notify("已退出登录");
    setTab("market");
  }

  async function deleteMcp(m: McpInfo) {
    if (!confirm(`删除 MCP「${m.name}」？`)) return;
    try {
      await api.deleteMcp(m.name);
      notify("已删除");
      load();
    } catch (e) {
      notify((e as Error).message);
    }
  }
  async function deleteSecret(key: string) {
    if (!confirm(`删除变量「${key}」？`)) return;
    try {
      await api.deleteSecret(key);
      notify("已删除");
      load();
    } catch (e) {
      notify((e as Error).message);
    }
  }

  const title = tab === "market" ? "应用市场" : tab === "mine" ? "我的 MCP" : "我的变量";
  const subtitle =
    tab === "market"
      ? "浏览所有公开的 MCP：每个声明运行拓扑与所需变量，可一键转 CLI 使用。"
      : tab === "mine"
        ? "你注册的全部 MCP（含私有）。设为 public 即上架市场。"
        : "渐进式凭据池（AES-256-GCM 加密）。MCP 清单里的 {VAR} 运行时按需缝合。";

  return (
    <div className="min-h-full">
      <Header
        tab={tab}
        onTab={switchTab}
        user={user}
        onLogin={() => setShowLogin(true)}
        onLogout={logout}
      />

      <main className="mx-auto max-w-6xl px-6 py-9 pb-16">
        <div className="flex flex-wrap items-end justify-between gap-6">
          <div>
            <h1 className="text-[28px] font-bold tracking-wide text-slate-800">{title}</h1>
            <p className="mt-2 max-w-2xl text-slate-400">{subtitle}</p>
          </div>

          {tab === "market" && (
            <div className="flex gap-2.5">
              <div className="relative">
                <SearchIcon className="pointer-events-none absolute left-3.5 top-1/2 -translate-y-1/2 text-slate-400" />
                <input
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  onKeyDown={(e) => e.key === "Enter" && setSearch(query)}
                  placeholder="搜索名称 / 描述"
                  className="w-72 max-w-[60vw] rounded-xl border border-slate-200 bg-white py-2.5 pl-10 pr-4 text-sm outline-none transition focus:border-indigo-300 focus:ring-4 focus:ring-indigo-500/10"
                />
              </div>
              <button
                onClick={() => setSearch(query)}
                className="rounded-xl bg-indigo-500 px-5 py-2.5 text-sm font-semibold text-white transition hover:bg-indigo-600"
              >
                搜索
              </button>
            </div>
          )}
          {tab === "mine" && (
            <button
              onClick={() => setMcpModal({ edit: null })}
              className="flex items-center gap-1.5 rounded-xl bg-indigo-500 px-5 py-2.5 text-sm font-semibold text-white transition hover:bg-indigo-600"
            >
              <PlusIcon /> 新建 MCP
            </button>
          )}
          {tab === "secrets" && (
            <button
              onClick={() => setSecretEdit({ key: null })}
              className="flex items-center gap-1.5 rounded-xl bg-indigo-500 px-5 py-2.5 text-sm font-semibold text-white transition hover:bg-indigo-600"
            >
              <PlusIcon /> 设置变量
            </button>
          )}
        </div>

        <div className="mt-8">
          {loading ? (
            <Empty>加载中…</Empty>
          ) : error ? (
            <Empty big="加载失败">{error}</Empty>
          ) : tab === "secrets" ? (
            secrets.length === 0 ? (
              <Empty big="还没有变量">点击 “设置变量” 添加，例如 AIKO_HUB_KEY。</Empty>
            ) : (
              <div className="grid grid-cols-[repeat(auto-fill,minmax(320px,1fr))] gap-5">
                {secrets.map((s) => (
                  <div
                    key={s.key}
                    className="flex flex-col rounded-2xl border border-slate-200/70 bg-white p-6 shadow-sm"
                  >
                    <div className="flex items-start gap-4">
                      <div className="grid size-12 flex-none place-items-center rounded-xl bg-gradient-to-br from-indigo-500 to-violet-500 text-white">
                        <KeyIcon />
                      </div>
                      <div className="min-w-0">
                        <span className="rounded-md bg-slate-100 px-2 py-0.5 font-mono text-sm font-semibold text-slate-600">
                          {s.key}
                        </span>
                        <div className="mt-1.5 text-xs text-slate-400">更新于 {s.updated_at}</div>
                      </div>
                    </div>
                    <div className="mt-4 flex items-center justify-between">
                      <span className="rounded-lg border border-slate-200 bg-slate-50 px-2.5 py-1 text-xs text-slate-500">
                        已加密
                      </span>
                      <div className="flex gap-2">
                        <button
                          onClick={() => setSecretEdit({ key: s.key })}
                          className="rounded-lg border border-indigo-200 px-3 py-1.5 text-xs font-semibold text-indigo-500 transition hover:bg-indigo-50"
                        >
                          修改
                        </button>
                        <button
                          onClick={() => deleteSecret(s.key)}
                          className="flex items-center rounded-lg border border-rose-200 px-2.5 py-1.5 text-xs font-semibold text-rose-500 transition hover:bg-rose-50"
                        >
                          <TrashIcon width={13} height={13} />
                        </button>
                      </div>
                    </div>
                  </div>
                ))}
              </div>
            )
          ) : items.length === 0 ? (
            tab === "market" ? (
              <Empty big="市场暂无公开的 MCP">登录后在 “我的 MCP” 创建并设为 public 即可上架。</Empty>
            ) : (
              <Empty big="你还没有注册任何 MCP">点击 “新建 MCP” 开始。</Empty>
            )
          ) : (
            <div className="grid grid-cols-[repeat(auto-fill,minmax(340px,1fr))] gap-5">
              {items.map((m) => (
                <McpCard
                  key={m.owner + "/" + m.name}
                  m={m}
                  mine={tab === "mine"}
                  onDetail={() => setDetail(m)}
                  onEdit={() => setMcpModal({ edit: m })}
                  onDelete={() => deleteMcp(m)}
                />
              ))}
            </div>
          )}
        </div>
      </main>

      {showLogin && <LoginModal onClose={() => setShowLogin(false)} onAuthed={onAuthed} />}
      {mcpModal && (
        <CreateMcpModal
          edit={mcpModal.edit}
          onClose={() => setMcpModal(null)}
          onSaved={(name) => {
            const editing = !!mcpModal.edit;
            setMcpModal(null);
            notify(editing ? "已更新 " + name : "已创建 " + name);
            load();
          }}
        />
      )}
      {detail && (
        <DetailModal
          m={detail}
          user={user}
          onClose={() => setDetail(null)}
          onRequireLogin={() => {
            setDetail(null);
            setShowLogin(true);
          }}
        />
      )}
      {secretEdit && (
        <SecretModal
          editKey={secretEdit.key}
          onClose={() => setSecretEdit(null)}
          onSaved={(key) => {
            setSecretEdit(null);
            notify("已保存 " + key);
            load();
          }}
        />
      )}

      {toast && (
        <div className="fixed bottom-7 left-1/2 z-[100] -translate-x-1/2 rounded-xl bg-slate-800 px-4 py-2.5 text-sm text-white shadow-2xl">
          {toast}
        </div>
      )}
    </div>
  );
}

function Empty({ big, children }: { big?: string; children: React.ReactNode }) {
  return (
    <div className="py-20 text-center text-slate-400">
      {big && <div className="mb-2 text-lg text-slate-500">{big}</div>}
      <div>{children}</div>
    </div>
  );
}
