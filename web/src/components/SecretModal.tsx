import { useState } from "react";
import Modal from "./Modal";
import { api } from "../lib/api";

const inputCls =
  "w-full rounded-xl border border-slate-200 px-3.5 py-2.5 text-sm outline-none transition focus:border-indigo-300 focus:ring-4 focus:ring-indigo-500/10";

export default function SecretModal({
  editKey,
  onClose,
  onSaved,
}: {
  editKey: string | null;
  onClose: () => void;
  onSaved: (key: string) => void;
}) {
  const [key, setKey] = useState(editKey ?? "");
  const [value, setValue] = useState("");
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);

  async function submit() {
    setErr("");
    if (!key.trim() || !value) return setErr("请填写变量名和值");
    setBusy(true);
    try {
      await api.setSecret(key.trim(), value);
      onSaved(key.trim());
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title={editKey ? "修改变量" : "设置变量"}
      subtitle="值经 AES-256-GCM 加密后存入凭据池，运行时按需缝合。"
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
            {busy ? "保存中…" : "保存"}
          </button>
        </>
      }
    >
      <div className="space-y-4">
        <div>
          <label className="mb-1.5 block text-xs font-semibold text-slate-600">变量名</label>
          <input
            className={`${inputCls} ${editKey ? "bg-slate-50 text-slate-400" : ""}`}
            value={key}
            readOnly={!!editKey}
            placeholder="AIKO_HUB_KEY"
            onChange={(e) => setKey(e.target.value)}
          />
        </div>
        <div>
          <label className="mb-1.5 block text-xs font-semibold text-slate-600">值</label>
          <input
            className={inputCls}
            type="password"
            value={value}
            autoFocus
            placeholder="••••••"
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && submit()}
          />
        </div>
        {err && <p className="text-sm text-rose-500">{err}</p>}
      </div>
    </Modal>
  );
}
