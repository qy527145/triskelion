import { useState } from "react";
import Modal from "./Modal";
import { api } from "../lib/api";

const inputCls =
  "w-full rounded-xl border border-slate-200 px-3.5 py-2.5 text-sm outline-none transition focus:border-indigo-300 focus:ring-4 focus:ring-indigo-500/10";

/** 编辑个人资料：目前仅昵称。留空表示清除，展示回退用户名。 */
export default function EditProfileModal({
  current,
  onClose,
  onSaved,
}: {
  current: string;
  onClose: () => void;
  onSaved: (displayName: string) => void;
}) {
  const [name, setName] = useState(current);
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);

  async function submit() {
    setErr("");
    const trimmed = name.trim();
    if (trimmed.length > 64) return setErr("昵称最长 64 个字符");
    setBusy(true);
    try {
      const r = await api.updateProfile(trimmed);
      onSaved(r.display_name);
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title="编辑资料"
      subtitle="昵称用于市场里的作者展示，也可按昵称检索作者。"
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
          <label className="mb-1.5 block text-xs font-semibold text-slate-600">昵称</label>
          <input
            className={inputCls}
            value={name}
            autoFocus
            placeholder="留空则展示用户名"
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && submit()}
          />
        </div>
        {err && <p className="text-sm text-rose-500">{err}</p>}
      </div>
    </Modal>
  );
}
