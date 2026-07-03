import type { ReactKind } from "../lib/types";
import { DownloadIcon, HeartIcon, StarIcon } from "./icons";

/**
 * 点赞 / 收藏切换 + 计数（可选展示下载量）。
 * 传入 onReact 时可交互（点击切换），缺省则只读展示。
 */
export default function ReactionBar({
  likes,
  favorites,
  downloads,
  liked,
  favorited,
  onReact,
}: {
  likes: number;
  favorites: number;
  /** 下载次数；undefined 表示该资源不适用（如 MCP）。 */
  downloads?: number;
  liked: boolean;
  favorited: boolean;
  onReact?: (kind: ReactKind) => void;
}) {
  const cls = (active: boolean, activeCls: string) =>
    `flex items-center gap-1 text-xs font-medium transition ${
      active ? activeCls : "text-slate-400" + (onReact ? " hover:text-slate-600" : "")
    } ${onReact ? "" : "cursor-default"}`;
  return (
    <div className="flex items-center gap-3">
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          onReact?.("like");
        }}
        className={cls(liked, "text-rose-500")}
        title={onReact ? (liked ? "取消点赞" : "点赞") : "点赞数"}
      >
        <HeartIcon width={14} height={14} fill={liked ? "currentColor" : "none"} /> {likes}
      </button>
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          onReact?.("favorite");
        }}
        className={cls(favorited, "text-amber-500")}
        title={onReact ? (favorited ? "取消收藏" : "收藏") : "收藏数"}
      >
        <StarIcon width={14} height={14} fill={favorited ? "currentColor" : "none"} /> {favorites}
      </button>
      {downloads !== undefined && (
        <span className="flex items-center gap-1 text-xs text-slate-400" title="下载次数">
          <DownloadIcon width={14} height={14} /> {downloads}
        </span>
      )}
    </div>
  );
}
