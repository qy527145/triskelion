import { useState } from "react";
import Modal from "./Modal";
import { api } from "../lib/api";

/**
 * 个人中心：把自己的技能 / MCP 转移给另一个用户。
 * 目标账号必须已存在；转移后当前用户失去该资源的所有权。
 */
export default function TransferModal({
  kind,
  owner,
  name,
  onClose,
  onDone,
}: {
  kind: "skill" | "mcp";
  owner: string;
  name: string;
  onClose: () => void;
  onDone: (msg: string) => void;
}) {
  const [target, setTarget] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const kindLabel = kind === "skill" ? "技能" : "MCP";

  async function submit() {
    const t = target.trim();
    if (!t) {
      setError("请输入接收方用户名");
      return;
    }
    if (!confirm(`确认将${kindLabel}「${name}」转移给「${t}」？\n转移后你将失去它的所有权。`)) return;
    setBusy(true);
    setError("");
    try {
      if (kind === "skill") await api.transferSkill(owner, name, t);
      else await api.transferMcp(name, t);
      onDone(`已将 ${name} 转移给 ${t}`);
    } catch (e) {
      setError((e as Error).message);
      setBusy(false);
    }
  }

  return (
    <Modal
      title={`转移${kindLabel}`}
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
            onClick={submit}
            disabled={busy}
            className="rounded-xl bg-indigo-500 px-4 py-2 text-sm font-semibold text-white transition hover:bg-indigo-600 disabled:opacity-60"
          >
            {busy ? "转移中…" : "确认转移"}
          </button>
        </>
      }
    >
      <div className="space-y-4">
        <label className="block">
          <span className="mb-1.5 block text-sm font-medium text-slate-600">接收方用户名</span>
          <input
            autoFocus
            value={target}
            onChange={(e) => setTarget(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && submit()}
            placeholder="对方必须是已注册用户"
            className="w-full rounded-xl border border-slate-200 bg-white px-3.5 py-2.5 text-sm outline-none transition focus:border-indigo-300 focus:ring-4 focus:ring-indigo-500/10"
          />
        </label>
        <p className="rounded-lg bg-amber-50 px-3 py-2 text-xs text-amber-600">
          转移后该{kindLabel}归对方所有，你将无法再编辑或删除；若对方已有同名{kindLabel}则会失败。
        </p>
        {error && <div className="text-sm text-rose-500">{error}</div>}
      </div>
    </Modal>
  );
}
