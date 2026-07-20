import { useEffect, useState } from "react";
import Modal from "./Modal";
import { api, ApiError, setAuth } from "../lib/api";
import type { AuthConfig } from "../lib/types";

const inputCls =
  "w-full rounded-xl border border-slate-200 px-3.5 py-2.5 text-sm outline-none transition focus:border-indigo-300 focus:ring-4 focus:ring-indigo-500/10";

export default function LoginModal({
  onClose,
  onAuthed,
}: {
  onClose: () => void;
  onAuthed: (username: string) => void;
}) {
  const [u, setU] = useState("");
  const [p, setP] = useState("");
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);
  // 认证能力探测：注册是否开放 / 是否有 LDAP。取不到（旧服务端）按开放注册处理。
  const [cfg, setCfg] = useState<AuthConfig | null>(null);

  useEffect(() => {
    api
      .authConfig()
      .then(setCfg)
      .catch(() => setCfg(null));
  }, []);

  const canRegister = cfg?.registration_enabled !== false;

  async function submit() {
    setErr("");
    if (!u.trim() || !p) {
      setErr("请填写用户名和密码");
      return;
    }
    setBusy(true);
    try {
      let resp;
      try {
        resp = await api.login(u.trim(), p);
      } catch (e) {
        // 注册开放时沿用「账号不存在即自动注册」；关闭后直接把 404 报给用户。
        if (e instanceof ApiError && e.status === 404 && canRegister) {
          resp = await api.register(u.trim(), p);
        } else {
          throw e;
        }
      }
      setAuth(resp.token, resp.username);
      onAuthed(resp.username);
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  const subtitle = canRegister
    ? "用户名密码登录，账号不存在将自动注册。"
    : cfg?.ldap_enabled
      ? "用户注册已关闭，请使用已有账号或 LDAP 账号登录。"
      : "用户注册已关闭，请使用已有账号登录。";

  return (
    <Modal
      title={canRegister ? "登录 / 注册" : "登录"}
      subtitle={subtitle}
      onClose={onClose}
      footer={
        <>
          <button
            onClick={onClose}
            className="rounded-xl border border-slate-200 px-4 py-2.5 text-sm font-semibold text-slate-600 hover:bg-slate-50"
          >
            取消
          </button>
          <button
            onClick={submit}
            disabled={busy}
            className="rounded-xl bg-indigo-500 px-5 py-2.5 text-sm font-semibold text-white hover:bg-indigo-600 disabled:opacity-60"
          >
            {busy ? "处理中…" : "登录"}
          </button>
        </>
      }
    >
      <div className="space-y-4">
        <div>
          <label className="mb-1.5 block text-xs font-semibold text-slate-600">用户名</label>
          <input
            className={inputCls}
            value={u}
            autoFocus
            placeholder="如 xuqiao"
            onChange={(e) => setU(e.target.value)}
          />
        </div>
        <div>
          <label className="mb-1.5 block text-xs font-semibold text-slate-600">密码</label>
          <input
            className={inputCls}
            type="password"
            value={p}
            placeholder={canRegister ? "至少 6 位" : "账号密码"}
            onChange={(e) => setP(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && submit()}
          />
        </div>
        {err && <p className="text-sm text-rose-500">{err}</p>}
      </div>
    </Modal>
  );
}
