/** 站点品牌标识：渐变三曲臂记号 + 可选文字。Header 与管理后台共用，保证观感一致。 */
export default function Brand({
  label = "triskelion",
  sub,
  size = "md",
}: {
  label?: string;
  sub?: string;
  size?: "sm" | "md";
}) {
  const box = size === "sm" ? "size-8 text-[15px]" : "size-9 text-base";
  return (
    <div className="flex items-center gap-2.5 text-lg font-bold text-slate-800">
      <span
        className={`grid ${box} place-items-center rounded-xl bg-gradient-to-br from-indigo-500 via-violet-500 to-sky-400 font-extrabold text-white shadow-sm shadow-indigo-500/30`}
      >
        T
      </span>
      <span className="flex items-baseline gap-2">
        {label}
        {sub && <span className="text-sm font-medium text-slate-400">{sub}</span>}
      </span>
    </div>
  );
}
