import { initials } from "../lib/color";
import { LogoutIcon } from "./icons";

export type Tab = "skill-market" | "mcp-market" | "skill-mine" | "mcp-mine" | "secrets";

/** 个人中心下的子页签。 */
export const PERSONAL_TABS: Tab[] = ["skill-mine", "mcp-mine", "secrets"];
export const isPersonal = (t: Tab) => PERSONAL_TABS.includes(t);

const MARKET_TABS: { id: Tab; label: string }[] = [
  { id: "skill-market", label: "技能市场" },
  { id: "mcp-market", label: "MCP 市场" },
];

export default function Header({
  tab,
  onTab,
  user,
  onLogin,
  onLogout,
}: {
  tab: Tab;
  onTab: (t: Tab) => void;
  user: string | null;
  onLogin: () => void;
  onLogout: () => void;
}) {
  const personalActive = isPersonal(tab);
  return (
    <header className="sticky top-0 z-40 flex items-center gap-7 border-b border-slate-200 bg-white/90 px-6 py-3.5 backdrop-blur">
      <div className="flex items-center gap-2.5 text-lg font-bold text-slate-800">
        <span className="grid size-9 place-items-center rounded-[10px] bg-gradient-to-br from-indigo-500 to-violet-500 font-extrabold text-white">
          T
        </span>
        triskelion
      </div>

      <nav className="flex gap-1">
        {MARKET_TABS.map((t) => (
          <button
            key={t.id}
            onClick={() => onTab(t.id)}
            className={`rounded-[10px] px-3 py-2 text-[15px] font-medium transition ${
              tab === t.id
                ? "bg-indigo-50 font-semibold text-indigo-500"
                : "text-slate-600 hover:bg-slate-100"
            }`}
          >
            {t.label}
          </button>
        ))}
        {user && (
          <button
            onClick={() => onTab("skill-mine")}
            className={`rounded-[10px] px-3 py-2 text-[15px] font-medium transition ${
              personalActive
                ? "bg-indigo-50 font-semibold text-indigo-500"
                : "text-slate-600 hover:bg-slate-100"
            }`}
          >
            个人中心
          </button>
        )}
      </nav>

      <div className="flex-1" />

      {user ? (
        <div className="flex items-center gap-3">
          <button
            onClick={() => onTab("skill-mine")}
            className="flex items-center gap-2 rounded-full border border-indigo-100 bg-indigo-50/50 py-1 pl-1 pr-3 transition hover:border-indigo-200 hover:bg-indigo-50"
            title="进入个人中心"
          >
            <span className="grid size-8 place-items-center rounded-full border border-indigo-200 bg-indigo-50 text-xs font-bold text-indigo-500">
              {initials(user)}
            </span>
            <span className="text-sm font-medium text-slate-700">{user}</span>
          </button>
          <button
            onClick={onLogout}
            className="flex items-center gap-1 rounded-lg px-2 py-1.5 text-sm text-slate-500 transition hover:bg-slate-100 hover:text-rose-500"
          >
            <LogoutIcon width={16} height={16} /> 退出
          </button>
        </div>
      ) : (
        <button
          onClick={onLogin}
          className="rounded-[10px] bg-indigo-500 px-4 py-2 text-sm font-semibold text-white transition hover:bg-indigo-600"
        >
          登录
        </button>
      )}
    </header>
  );
}
