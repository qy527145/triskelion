import type { McpInfo, McpManifest, SecretInfo } from "./types";

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

async function req<T>(path: string, opts: ReqOpts = {}): Promise<T> {
  const { method = "GET", body, auth = false } = opts;
  const headers: Record<string, string> = {};
  if (body !== undefined) headers["Content-Type"] = "application/json";
  const token = getToken();
  if (auth && token) headers["Authorization"] = "Bearer " + token;

  let res: Response;
  try {
    res = await fetch(path, {
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

export interface AuthResp {
  token: string;
  username: string;
}

export const api = {
  login: (username: string, password: string) =>
    req<AuthResp>("/v1/auth/login", { method: "POST", body: { username, password } }),
  register: (username: string, password: string) =>
    req<AuthResp>("/v1/auth/register", { method: "POST", body: { username, password } }),

  explore: (q: string) =>
    req<McpInfo[]>("/v1/explore" + (q ? "?q=" + encodeURIComponent(q) : "")),
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
};
