import type {
  AdminGroup,
  AdminLabel,
  AdminMcp,
  AdminSkill,
  AdminStats,
  AdminUser,
  BatchResult,
  CallsQuery,
  CallsResp,
  FavoritesResp,
  GroupVisibility,
  ImportSummary,
  McpInfo,
  McpManifest,
  ReactKind,
  ReactResp,
  SecretInfo,
  SkillInfo,
  SkillInspectResp,
  SkillManifest,
  SkillVersionDeleteResp,
  SkillVersionInfo,
  UserTransferResult,
} from "./types";

const TOKEN_KEY = "tsk_token";
const USER_KEY = "tsk_user";

export function getToken(): string | null {
  return localStorage.getItem(TOKEN_KEY);
}
export function getUser(): string | null {
  return localStorage.getItem(USER_KEY);
}
export function setAuth(token: string, username: string) {
  localStorage.setItem(TOKEN_KEY, token);
  localStorage.setItem(USER_KEY, username);
}
export function clearAuth() {
  localStorage.removeItem(TOKEN_KEY);
  localStorage.removeItem(USER_KEY);
}

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

interface ReqOpts {
  method?: string;
  body?: unknown;
  auth?: boolean;
}

/**
 * 去掉开头的 `/`，把绝对路径转成相对路径，使前端可整体部署在 nginx 子路径下（如 `/triskelion/`）。
 * 浏览器以 `document.baseURI`（当前页面地址）为基准解析相对地址，
 * 因此请通过带末尾斜杠的地址访问（nginx 子路径通常会自动 301 补全斜杠）。
 */
function rel(path: string): string {
  return path.replace(/^\/+/, "");
}

async function req<T>(path: string, opts: ReqOpts = {}): Promise<T> {
  const { method = "GET", body, auth = false } = opts;
  const headers: Record<string, string> = {};
  if (body !== undefined) headers["Content-Type"] = "application/json";
  const token = getToken();
  if (auth && token) headers["Authorization"] = "Bearer " + token;

  let res: Response;
  try {
    res = await fetch(rel(path), {
      method,
      headers,
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });
  } catch (e) {
    throw new ApiError(0, "无法连接 Hub：" + (e as Error).message);
  }

  if (res.status === 204) return null as T;
  const text = await res.text();
  let data: unknown = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = { error: text };
  }
  if (!res.ok) {
    const msg =
      (data && typeof data === "object" && "error" in data
        ? (data as { error: string }).error
        : null) ?? res.statusText;
    throw new ApiError(res.status, msg);
  }
  return data as T;
}

/**
 * 上传原始二进制体（如压缩包），带 Bearer 鉴权，响应体按 JSON 解析。
 * 与 `req` 的区别：body 直接是字节流，不做 JSON 序列化、不设 application/json。
 */
async function reqRaw<T>(path: string, body: BodyInit): Promise<T> {
  const headers: Record<string, string> = { "Content-Type": "application/octet-stream" };
  const token = getToken();
  if (token) headers["Authorization"] = "Bearer " + token;

  let res: Response;
  try {
    res = await fetch(rel(path), { method: "POST", headers, body });
  } catch (e) {
    throw new ApiError(0, "无法连接 Hub：" + (e as Error).message);
  }

  if (res.status === 204) return null as T;
  const text = await res.text();
  let data: unknown = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = { error: text };
  }
  if (!res.ok) {
    const msg =
      (data && typeof data === "object" && "error" in data
        ? (data as { error: string }).error
        : null) ?? res.statusText;
    throw new ApiError(res.status, msg);
  }
  return data as T;
}

export interface AuthResp {
  token: string;
  username: string;
}

export const api = {
  login: (username: string, password: string) =>
    req<AuthResp>("/v1/auth/login", { method: "POST", body: { username, password } }),
  register: (username: string, password: string) =>
    req<AuthResp>("/v1/auth/register", { method: "POST", body: { username, password } }),

  explore: (q: string, label?: string) => {
    const p = new URLSearchParams();
    if (q) p.set("q", q);
    if (label) p.set("label", label);
    const qs = p.toString();
    return req<McpInfo[]>("/v1/explore" + (qs ? "?" + qs : ""));
  },
  listMine: () => req<McpInfo[]>("/v1/mcp", { auth: true }),
  upsertMcp: (manifest: McpManifest, visibility: string) =>
    req<McpInfo>("/v1/mcp", { method: "POST", auth: true, body: { manifest, visibility } }),
  renameMcp: (name: string, newName: string) =>
    req<McpInfo>("/v1/mcp/" + encodeURIComponent(name) + "/rename", {
      method: "POST",
      auth: true,
      body: { new_name: newName },
    }),
  deleteMcp: (name: string) =>
    req<null>("/v1/mcp/" + encodeURIComponent(name), { method: "DELETE", auth: true }),
  /** 点赞 / 收藏一个 MCP（或取消），返回最新计数与查看者标记。 */
  reactMcp: (owner: string, name: string, kind: ReactKind, on: boolean) =>
    req<ReactResp>(
      "/v1/mcp/" + encodeURIComponent(owner) + "/" + encodeURIComponent(name) + "/react",
      { method: "POST", auth: true, body: { kind, on } },
    ),
  /** 把自己的 MCP 转移给另一个用户。 */
  transferMcp: (name: string, newOwner: string) =>
    req<null>("/v1/mcp/" + encodeURIComponent(name) + "/transfer", {
      method: "POST",
      auth: true,
      body: { new_owner: newOwner },
    }),

  callTool: (owner: string, name: string, tool: string, args: unknown) =>
    req<unknown>(
      "/v1/run/" + encodeURIComponent(owner) + "/" + encodeURIComponent(name) + "/call",
      { method: "POST", auth: true, body: { tool, arguments: args } },
    ),

  listSecrets: () => req<SecretInfo[]>("/v1/secret", { auth: true }),
  setSecret: (key: string, value: string) =>
    req<SecretInfo>("/v1/secret", { method: "PUT", auth: true, body: { key, value } }),
  deleteSecret: (key: string) =>
    req<null>("/v1/secret/" + encodeURIComponent(key), { method: "DELETE", auth: true }),

  // 技能市场
  skillExplore: (q: string, category?: string, tag?: string, label?: string) => {
    const p = new URLSearchParams();
    if (q) p.set("q", q);
    if (category) p.set("category", category);
    if (tag) p.set("tag", tag);
    if (label) p.set("label", label);
    const qs = p.toString();
    return req<SkillInfo[]>("/v1/skill/explore" + (qs ? "?" + qs : ""));
  },
  listMySkills: () => req<SkillInfo[]>("/v1/skill", { auth: true }),
  /** 技能详情。`version` 指定历史版本（缺省最新版）；响应 versions 列出全部版本（新→旧）。 */
  getSkill: (owner: string, name: string, version?: string) =>
    req<SkillInfo>(
      "/v1/skill/" +
        encodeURIComponent(owner) +
        "/" +
        encodeURIComponent(name) +
        (version ? "?version=" + encodeURIComponent(version) : ""),
      { auth: true },
    ),
  /** 技能已发布的全部版本副本（新→旧）。 */
  skillVersions: (owner: string, name: string) =>
    req<SkillVersionInfo[]>(
      "/v1/skill/" + encodeURIComponent(owner) + "/" + encodeURIComponent(name) + "/versions",
      { auth: true },
    ),
  upsertSkill: (
    manifest: SkillManifest,
    visibility: string,
    skill_md: string,
    archive_sha256 = "",
    archive_size = 0,
  ) =>
    req<SkillInfo>("/v1/skill", {
      method: "POST",
      auth: true,
      body: { manifest, visibility, skill_md, archive_sha256, archive_size },
    }),
  /**
   * 拖入压缩包创建技能：上传原始压缩包字节（zip / tar.zst / tar.gz / 裸 tar），
   * 服务端解包归一化后回吐清单 + 说明书 + 已落盘的压缩体 sha256/size，供表单预填确认。
   */
  inspectSkillArchive: (file: Blob) =>
    reqRaw<SkillInspectResp>("/v1/skill/inspect", file),
  renameSkill: (owner: string, name: string, newName: string) =>
    req<SkillInfo>(
      "/v1/skill/" + encodeURIComponent(owner) + "/" + encodeURIComponent(name) + "/rename",
      { method: "POST", auth: true, body: { new_name: newName } },
    ),
  deleteSkill: (owner: string, name: string) =>
    req<null>("/v1/skill/" + encodeURIComponent(owner) + "/" + encodeURIComponent(name), {
      method: "DELETE",
      auth: true,
    }),
  /** 点赞 / 收藏一个技能（或取消），返回最新计数与查看者标记。 */
  reactSkill: (owner: string, name: string, kind: ReactKind, on: boolean) =>
    req<ReactResp>(
      "/v1/skill/" + encodeURIComponent(owner) + "/" + encodeURIComponent(name) + "/react",
      { method: "POST", auth: true, body: { kind, on } },
    ),
  /** 把自己的技能转移给另一个用户。 */
  transferSkill: (owner: string, name: string, newOwner: string) =>
    req<null>(
      "/v1/skill/" + encodeURIComponent(owner) + "/" + encodeURIComponent(name) + "/transfer",
      { method: "POST", auth: true, body: { new_owner: newOwner } },
    ),
  /** 当前用户收藏的全部资源（技能 + MCP）。 */
  favorites: () => req<FavoritesResp>("/v1/favorites", { auth: true }),
  /** 压缩体下载地址。`version` 指定历史版本（缺省最新版）。 */
  skillArchiveUrl: (owner: string, name: string, version?: string) =>
    rel(
      "/v1/skill/" +
        encodeURIComponent(owner) +
        "/" +
        encodeURIComponent(name) +
        "/archive" +
        (version ? "?version=" + encodeURIComponent(version) : ""),
    ),

  /** 公开受管标签名清单（供市场筛选）。 */
  listLabels: () => req<string[]>("/v1/labels"),
};

// ---------------------------------------------------------------------------
// 管理后台（ADMIN_TOKEN 鉴权，请求头 X-Admin-Token）
// ---------------------------------------------------------------------------

async function adminReq<T>(
  path: string,
  token: string,
  opts: { method?: string; body?: unknown } = {},
): Promise<T> {
  const { method = "GET", body } = opts;
  const headers: Record<string, string> = { "X-Admin-Token": token };
  if (body !== undefined) headers["Content-Type"] = "application/json";
  let res: Response;
  try {
    res = await fetch(rel(path), {
      method,
      headers,
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });
  } catch (e) {
    throw new ApiError(0, "无法连接 Hub：" + (e as Error).message);
  }
  if (res.status === 204) return null as T;
  const text = await res.text();
  let data: unknown = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = { error: text };
  }
  if (!res.ok) {
    const msg =
      (data && typeof data === "object" && "error" in data
        ? (data as { error: string }).error
        : null) ?? res.statusText;
    throw new ApiError(res.status, msg);
  }
  return data as T;
}

export const admin = {
  stats: (token: string) => adminReq<AdminStats>("/v1/admin/stats", token),
  users: (token: string) => adminReq<AdminUser[]>("/v1/admin/users", token),
  skills: (token: string) => adminReq<AdminSkill[]>("/v1/admin/skills", token),
  mcps: (token: string) => adminReq<AdminMcp[]>("/v1/admin/mcps", token),
  calls: (token: string, params: CallsQuery = {}) => {
    const p = new URLSearchParams();
    if (params.service) p.set("service", params.service);
    if (params.tool) p.set("tool", params.tool);
    if (params.caller) p.set("caller", params.caller);
    if (params.window) p.set("window", String(params.window));
    if (params.errors_only) p.set("errors_only", "true");
    if (params.limit != null) p.set("limit", String(params.limit));
    if (params.offset != null) p.set("offset", String(params.offset));
    const qs = p.toString();
    return adminReq<CallsResp>("/v1/admin/calls" + (qs ? "?" + qs : ""), token);
  },

  // 分组 CRUD
  groups: (token: string) => adminReq<AdminGroup[]>("/v1/admin/groups", token),
  createGroup: (token: string, name: string, description: string) =>
    adminReq<AdminGroup>("/v1/admin/groups", token, {
      method: "POST",
      body: { name, description },
    }),
  updateGroup: (token: string, id: number, body: { name?: string; description?: string }) =>
    adminReq<null>("/v1/admin/groups/" + id, token, { method: "PATCH", body }),
  deleteGroup: (token: string, id: number) =>
    adminReq<null>("/v1/admin/groups/" + id, token, { method: "DELETE" }),

  // 标签 CRUD
  labels: (token: string) => adminReq<AdminLabel[]>("/v1/admin/labels", token),
  createLabel: (token: string, name: string) =>
    adminReq<AdminLabel>("/v1/admin/labels", token, { method: "POST", body: { name } }),
  updateLabel: (token: string, id: number, name: string) =>
    adminReq<null>("/v1/admin/labels/" + id, token, { method: "PATCH", body: { name } }),
  deleteLabel: (token: string, id: number) =>
    adminReq<null>("/v1/admin/labels/" + id, token, { method: "DELETE" }),

  // 用户 CRUD
  createUser: (token: string, username: string, password: string, group_ids: number[]) =>
    adminReq<null>("/v1/admin/users", token, {
      method: "POST",
      body: { username, password, group_ids },
    }),
  updateUser: (
    token: string,
    id: number,
    body: { password?: string; group_ids: number[] },
  ) => adminReq<null>("/v1/admin/users/" + id, token, { method: "PATCH", body }),
  deleteUser: (token: string, id: number) =>
    adminReq<null>("/v1/admin/users/" + id, token, { method: "DELETE" }),

  // 市场资源（技能 / MCP）可见性配置 + 内容编辑 + 删除
  updateSkill: (
    token: string,
    owner: string,
    name: string,
    body: {
      visibility?: string;
      group_visibility?: GroupVisibility;
      label_ids?: number[];
      version?: string;
      category?: string;
      description?: string;
      tags?: string[];
      skill_md?: string;
      mcp_dependencies?: string[];
      preferred_tools?: string[];
    },
  ) =>
    adminReq<null>(
      "/v1/admin/skills/" + encodeURIComponent(owner) + "/" + encodeURIComponent(name),
      token,
      { method: "PATCH", body },
    ),
  deleteSkill: (token: string, owner: string, name: string) =>
    adminReq<null>(
      "/v1/admin/skills/" + encodeURIComponent(owner) + "/" + encodeURIComponent(name),
      token,
      { method: "DELETE" },
    ),
  /** 某技能已发布的全部版本副本（新→旧）。 */
  skillVersions: (token: string, owner: string, name: string) =>
    adminReq<SkillVersionInfo[]>(
      "/v1/admin/skills/" +
        encodeURIComponent(owner) +
        "/" +
        encodeURIComponent(name) +
        "/versions",
      token,
    ),
  /**
   * 删除技能的指定版本副本。删除最新版会自动把次新版本提升为最新版；
   * 仅剩最后一个版本时服务端拒绝。返回删除后的最新版本号与剩余版本列表。
   */
  deleteSkillVersion: (token: string, owner: string, name: string, version: string) =>
    adminReq<SkillVersionDeleteResp>(
      "/v1/admin/skills/" +
        encodeURIComponent(owner) +
        "/" +
        encodeURIComponent(name) +
        "/versions/" +
        encodeURIComponent(version),
      token,
      { method: "DELETE" },
    ),
  updateMcp: (
    token: string,
    owner: string,
    name: string,
    body: {
      visibility?: string;
      group_visibility?: GroupVisibility;
      label_ids?: number[];
      manifest?: McpManifest;
    },
  ) =>
    adminReq<null>(
      "/v1/admin/mcps/" + encodeURIComponent(owner) + "/" + encodeURIComponent(name),
      token,
      { method: "PATCH", body },
    ),
  deleteMcp: (token: string, owner: string, name: string) =>
    adminReq<null>(
      "/v1/admin/mcps/" + encodeURIComponent(owner) + "/" + encodeURIComponent(name),
      token,
      { method: "DELETE" },
    ),

  /** 批量配置技能 / MCP：可见性、可见分组、增删受管标签。 */
  batchUpdate: (
    token: string,
    body: {
      kind: "skill" | "mcp";
      targets: { owner: string; name: string }[];
      visibility?: string;
      group_visibility?: GroupVisibility;
      add_label_ids?: number[];
      remove_label_ids?: number[];
    },
  ) => adminReq<BatchResult>("/v1/admin/batch", token, { method: "POST", body }),

  /** 批量把选中的技能 / MCP 转移给另一个用户。 */
  transferResources: (
    token: string,
    body: {
      kind: "skill" | "mcp";
      targets: { owner: string; name: string }[];
      to_username: string;
    },
  ) => adminReq<BatchResult>("/v1/admin/transfer", token, { method: "POST", body }),

  /** 整户转移：把某用户名下全部技能与 MCP 转给另一个用户（注销前的资产交接）。 */
  transferUser: (token: string, id: number, to_username: string) =>
    adminReq<UserTransferResult>("/v1/admin/users/" + id + "/transfer", token, {
      method: "POST",
      body: { to_username },
    }),

  /** 导出全量资源包，触发浏览器下载 .tskpack。 */
  exportPack: async (token: string): Promise<void> => {
    const res = await fetch(rel("/v1/admin/export"), { headers: { "X-Admin-Token": token } });
    if (!res.ok) {
      const text = await res.text();
      let msg = res.statusText;
      try {
        msg = (JSON.parse(text) as { error?: string }).error ?? msg;
      } catch {
        /* keep statusText */
      }
      throw new ApiError(res.status, msg);
    }
    const disposition = res.headers.get("Content-Disposition") ?? "";
    const match = /filename="?([^"]+)"?/.exec(disposition);
    const filename = match?.[1] ?? "triskelion-export.tskpack";
    const blob = await res.blob();
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
  },

  /** 导入全量资源包（上传 .tskpack 原始字节）。 */
  importPack: async (token: string, file: File): Promise<ImportSummary> => {
    let res: Response;
    try {
      res = await fetch(rel("/v1/admin/import"), {
        method: "POST",
        headers: { "X-Admin-Token": token, "Content-Type": "application/zstd" },
        body: file,
      });
    } catch (e) {
      throw new ApiError(0, "无法连接 Hub：" + (e as Error).message);
    }
    const text = await res.text();
    let data: unknown = null;
    try {
      data = text ? JSON.parse(text) : null;
    } catch {
      data = { error: text };
    }
    if (!res.ok) {
      const msg =
        (data && typeof data === "object" && "error" in data
          ? (data as { error: string }).error
          : null) ?? res.statusText;
      throw new ApiError(res.status, msg);
    }
    return data as ImportSummary;
  },
};
