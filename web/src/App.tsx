import { useCallback, useEffect, useState } from "react";
import Header, { type Tab, isPersonal } from "./components/Header";
import McpCard from "./components/McpCard";
import SkillCard from "./components/SkillCard";
import LoginModal from "./components/LoginModal";
import CreateMcpModal from "./components/CreateMcpModal";
import CreateSkillModal from "./components/CreateSkillModal";
import DetailModal from "./components/DetailModal";
import SkillDetailModal from "./components/SkillDetailModal";
import SecretModal from "./components/SecretModal";
import { SearchIcon, PlusIcon, KeyIcon, TrashIcon, Spinner } from "./components/icons";
import { api, clearAuth, getUser } from "./lib/api";
import type { McpInfo, SecretInfo, SkillInfo } from "./lib/types";
import { SKILL_CATEGORIES } from "./lib/types";

const isSkillTab = (t: Tab) => t === "skill-market" || t === "skill-mine";
const isMcpTab = (t: Tab) => t === "mcp-market" || t === "mcp-mine";
const isMarket = (t: Tab) => t === "skill-market" || t === "mcp-market";

const PERSONAL: { id: Tab; label: string }[] = [
  { id: "skill-mine", label: "我的技能" },
  { id: "mcp-mine", label: "我的 MCP" },
  { id: "secrets", label: "我的变量" },
];

export default function App() {
  const [user, setUser] = useState<string | null>(getUser());
  const [tab, setTab] = useState<Tab>("skill-market");

  const [items, setItems] = useState<McpInfo[]>([]);
  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [secrets, setSecrets] = useState<SecretInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [query, setQuery] = useState("");
  const [search, setSearch] = useState("");
  const [category, setCategory] = useState(""); // 技能市场分类过滤
  const [label, setLabel] = useState(""); // 市场受管标签过滤（官方/社区等）
  const [labelOptions, setLabelOptions] = useState<string[]>([]);

  const [showLogin, setShowLogin] = useState(false);
  const [mcpModal, setMcpModal] = useState<{ edit: McpInfo | null } | null>(null);
  const [skillModal, setSkillModal] = useState<{ edit: SkillInfo | null } | null>(null);
  const [detail, setDetail] = useState<McpInfo | null>(null);
  const [skillDetail, setSkillDetail] = useState<SkillInfo | null>(null);
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
      } else if (tab === "skill-market") {
        setSkills(await api.skillExplore(search, category, undefined, label));
      } else if (tab === "skill-mine") {
        setSkills(await api.listMySkills());
      } else if (tab === "mcp-mine") {
        setItems(await api.listMine());
      } else {
        setItems(await api.explore(search, label));
      }
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  }, [tab, search, category, label]);

  useEffect(() => {
    load();
  }, [load]);

  // 受管标签清单（供市场筛选）。
  useEffect(() => {
    api.listLabels().then(setLabelOptions).catch(() => setLabelOptions([]));
  }, []);

  function switchTab(t: Tab) {
    if ((t === "skill-mine" || t === "mcp-mine" || t === "secrets") && !user) {
      setShowLogin(true);
      return;
    }
    setQuery("");
    setSearch("");
    setCategory("");
    setLabel("");
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
    setTab("skill-market");
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
  async function deleteSkill(s: SkillInfo) {
    if (!confirm(`删除技能「${s.name}」？`)) return;
    try {
      await api.deleteSkill(s.owner, s.name);
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

  const meta: Record<Tab, { title: string; subtitle: string }> = {
    "skill-market": {
      title: "技能市场",
      subtitle: "万物皆 Skill：技能 / 知识库 / 工具链。Agent 只读精简 SKILL.md，按需用 tsk 触发底层 MCP。",
    },
    "mcp-market": {
      title: "MCP 市场",
      subtitle: "浏览所有公开 MCP：每个声明运行拓扑与所需变量，可一键转 CLI 使用。",
    },
    "skill-mine": {
      title: "我的技能",
      subtitle:
        "你发布的全部技能（含私有）。大文件夹技能用 tsk skill publish 上传；批量导入第三方生态用 tsk import <目录>（默认归类「社区资源」）；纯文本可在此直接新建。",
    },
    "mcp-mine": {
      title: "我的 MCP",
      subtitle: "你注册的全部 MCP（含私有）。设为 public 即上架市场。",
    },
    secrets: {
      title: "我的变量",
      subtitle: "渐进式凭据池（AES-256-GCM 加密）。MCP 清单里的 {VAR} 运行时按需缝合。",
    },
  };

  return (
    <div className="min-h-full">
      <Header tab={tab} onTab={switchTab} user={user} onLogin={() => setShowLogin(true)} onLogout={logout} />

      <main className="mx-auto max-w-6xl px-6 py-9 pb-16">
        <div className="flex flex-wrap items-end justify-between gap-6">
          <div>
            {isPersonal(tab) && (
              <div className="mb-1 text-xs font-semibold uppercase tracking-wider text-indigo-400">
                个人中心
              </div>
            )}
            <h1 className="text-[28px] font-bold tracking-wide text-slate-800">{meta[tab].title}</h1>
            <p className="mt-2 max-w-2xl text-slate-400">{meta[tab].subtitle}</p>
          </div>

          {isMarket(tab) && (
            <div className="flex gap-2.5">
              <div className="relative">
                <SearchIcon className="pointer-events-none absolute left-3.5 top-1/2 -translate-y-1/2 text-slate-400" />
                <input
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  onKeyDown={(e) => e.key === "Enter" && setSearch(query)}
                  placeholder={tab === "skill-market" ? "搜索名称 / 描述 / 标签" : "搜索名称 / 描述"}
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
          {tab === "skill-mine" && (
            <button
              onClick={() => setSkillModal({ edit: null })}
              className="flex items-center gap-1.5 rounded-xl bg-indigo-500 px-5 py-2.5 text-sm font-semibold text-white transition hover:bg-indigo-600"
            >
              <PlusIcon /> 新建技能
            </button>
          )}
          {tab === "mcp-mine" && (
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

        {isPersonal(tab) && (
          <div className="mt-6 flex flex-wrap gap-2">
            {PERSONAL.map((p) => (
              <Chip key={p.id} active={tab === p.id} onClick={() => switchTab(p.id)}>
                {p.label}
              </Chip>
            ))}
          </div>
        )}

        {tab === "skill-market" && (
          <div className="mt-6 flex flex-wrap gap-2">
            <Chip active={category === ""} onClick={() => setCategory("")}>
              全部
            </Chip>
            {SKILL_CATEGORIES.map((c) => (
              <Chip key={c.id} active={category === c.id} onClick={() => setCategory(c.id)}>
                {c.label}
              </Chip>
            ))}
          </div>
        )}

        {isMarket(tab) && labelOptions.length > 0 && (
          <div className="mt-3 flex flex-wrap items-center gap-2">
            <span className="mr-1 text-xs font-medium uppercase tracking-wider text-slate-400">标签</span>
            <Chip active={label === ""} onClick={() => setLabel("")}>
              全部
            </Chip>
            {labelOptions.map((l) => (
              <Chip key={l} active={label === l} onClick={() => setLabel(l)}>
                {l}
              </Chip>
            ))}
          </div>
        )}

        <div className="mt-8">
          {loading ? (
            <Loading />
          ) : error ? (
            <Empty big="加载失败">{error}</Empty>
          ) : tab === "secrets" ? (
            secrets.length === 0 ? (
              <Empty big="还没有变量">点击 “设置变量” 添加，例如 AIKO_HUB_KEY。</Empty>
            ) : (
              <div className="grid grid-cols-[repeat(auto-fill,minmax(320px,1fr))] gap-5 tsk-rise">
                {secrets.map((s) => (
                  <div
                    key={s.key}
                    className="flex flex-col rounded-2xl border border-slate-200/70 bg-white p-6 shadow-sm transition hover:-translate-y-0.5 hover:border-indigo-200 hover:shadow-lg hover:shadow-indigo-500/10"
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
          ) : isSkillTab(tab) ? (
            skills.length === 0 ? (
              tab === "skill-market" ? (
                <Empty big="技能市场暂无公开技能">登录后在 “我的技能” 创建，或用 tsk skill publish 发布并设为 public。</Empty>
              ) : (
                <Empty big="你还没有发布任何技能">点击 “新建技能”，或在本地 tsk skill init && tsk skill publish。</Empty>
              )
            ) : (
              <div className="grid grid-cols-[repeat(auto-fill,minmax(340px,1fr))] gap-5 tsk-rise">
                {skills.map((s) => (
                  <SkillCard
                    key={s.owner + "/" + s.name}
                    s={s}
                    mine={tab === "skill-mine"}
                    onDetail={() => setSkillDetail(s)}
                    onEdit={() => setSkillModal({ edit: s })}
                    onDelete={() => deleteSkill(s)}
                  />
                ))}
              </div>
            )
          ) : isMcpTab(tab) && items.length === 0 ? (
            tab === "mcp-market" ? (
              <Empty big="市场暂无公开的 MCP">登录后在 “我的 MCP” 创建并设为 public 即可上架。</Empty>
            ) : (
              <Empty big="你还没有注册任何 MCP">点击 “新建 MCP” 开始。</Empty>
            )
          ) : (
            <div className="grid grid-cols-[repeat(auto-fill,minmax(340px,1fr))] gap-5 tsk-rise">
              {items.map((m) => (
                <McpCard
                  key={m.owner + "/" + m.name}
                  m={m}
                  mine={tab === "mcp-mine"}
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
      {skillModal && (
        <CreateSkillModal
          edit={skillModal.edit}
          onClose={() => setSkillModal(null)}
          onSaved={(name) => {
            const editing = !!skillModal.edit;
            setSkillModal(null);
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
      {skillDetail && <SkillDetailModal s={skillDetail} onClose={() => setSkillDetail(null)} />}
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

function Chip({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      onClick={onClick}
      className={`rounded-full px-4 py-1.5 text-sm font-medium transition ${
        active
          ? "bg-indigo-500 text-white shadow-sm"
          : "border border-slate-200 bg-white text-slate-600 hover:bg-slate-50"
      }`}
    >
      {children}
    </button>
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

function Loading() {
  return (
    <div className="flex items-center justify-center gap-2.5 py-20 text-slate-400">
      <Spinner width={20} height={20} /> 加载中…
    </div>
  );
}
