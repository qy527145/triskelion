import { useState } from "react";
import Modal from "./Modal";
import { api, ApiError, setAuth } from "../lib/api";

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
        if (e instanceof ApiError && e.status === 404) {
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

  return (
    <Modal
      title="登录 / 注册"
      subtitle="用户名密码登录，账号不存在将自动注册。"
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
            placeholder="至少 6 位"
            onChange={(e) => setP(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && submit()}
          />
        </div>
        {err && <p className="text-sm text-rose-500">{err}</p>}
      </div>
    </Modal>
  );
}
