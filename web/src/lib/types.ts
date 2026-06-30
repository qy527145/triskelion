export type Runtime = "local" | "remote";
export type Protocol = "stdio" | "sse" | "streamable";

export interface McpManifest {
  resource_type: string;
  name: string;
  description: string;
  version: string;
  runtime: Runtime;
  protocol: Protocol;
  url?: string;
  command?: string;
  env?: Record<string, string>;
  headers?: Record<string, string>;
}

export interface JsonSchema {
  type?: string;
  description?: string;
  properties?: Record<string, JsonSchema>;
  required?: string[];
  items?: JsonSchema;
}

export interface ToolMeta {
  name: string;
  description: string;
  input_schema?: JsonSchema;
}

export interface McpInfo {
  owner: string;
  name: string;
  visibility: string;
  version: string;
  manifest: McpManifest;
  tools?: ToolMeta[];
  updated_at: string;
}

export interface SecretInfo {
  key: string;
  updated_at: string;
}

// --------------------------------------------------------------------------
// 技能市场（Skill marketplace）
// --------------------------------------------------------------------------

/** 逻辑分类：万物皆 Skill，category 仅是分类标签。 */
export type SkillCategory = "skill" | "kb" | "toolchain";

export const SKILL_CATEGORIES: { id: SkillCategory; label: string }[] = [
  { id: "skill", label: "技能" },
  { id: "kb", label: "知识库" },
  { id: "toolchain", label: "工具链" },
];

export function categoryLabel(c: string): string {
  return SKILL_CATEGORIES.find((x) => x.id === c)?.label ?? c;
}

export interface SkillManifest {
  name: string;
  version: string;
  category: string;
  description: string;
  tags: string[];
  mcp_dependencies: string[];
  preferred_tools: string[];
}

export interface SkillInfo {
  owner: string;
  name: string;
  category: string;
  visibility: string;
  version: string;
  description: string;
  tags: string[];
  mcp_dependencies: string[];
  preferred_tools: string[];
  skill_md: string;
  archive_sha256: string;
  archive_size: number;
  updated_at: string;
}

export function humanSize(n: number): string {
  if (!n) return "—";
  const u = ["B", "KB", "MB", "GB"];
  let f = n;
  let i = 0;
  while (f >= 1024 && i < u.length - 1) {
    f /= 1024;
    i++;
  }
  return i === 0 ? `${n} B` : `${f.toFixed(1)} ${u[i]}`;
}

// --------------------------------------------------------------------------
// 管理后台（admin）
// --------------------------------------------------------------------------

export interface AdminStats {
  users: number;
  skills: number;
  skills_public: number;
  mcps: number;
  mcps_public: number;
  secrets: number;
  blobs: number;
  blobs_bytes: number;
  calls_total: number;
  calls_24h: number;
  calls_errors_24h: number;
  top_tools: { tool: string; count: number }[];
  recent_errors: { tool: string; caller: string; error: string; at: string }[];
  admin_enabled: boolean;
  generated_at: string;
}

export interface AdminUser {
  username: string;
  created_at: string;
  skills: number;
  mcps: number;
  secrets: number;
}

export interface AdminSkill {
  owner: string;
  name: string;
  category: string;
  visibility: string;
  version: string;
  description: string;
  archive_size: number;
  has_archive: boolean;
  updated_at: string;
}

export interface AdminMcp {
  owner: string;
  name: string;
  visibility: string;
  version: string;
  runtime: string;
  protocol: string;
  updated_at: string;
}

export interface CallLog {
  caller: string;
  owner: string;
  mcp_name: string;
  tool: string;
  ok: boolean;
  error: string;
  ms: number;
  created_at: string;
}

export interface ImportSummary {
  users: number;
  mcps: number;
  skills: number;
  secrets: number;
  calls: number;
  blobs: number;
  skipped: string[];
}

/** 扫描清单里的 {VAR} 占位符（与服务端 required_vars 等价）。 */
export function requiredVars(m: McpManifest): string[] {
  const found: string[] = [];
  const scan = (s?: string) => {
    if (!s) return;
    const re = /\{([A-Za-z0-9_]+)\}/g;
    let mm: RegExpExecArray | null;
    while ((mm = re.exec(s))) if (!found.includes(mm[1])) found.push(mm[1]);
  };
  scan(m.url);
  scan(m.command);
  Object.values(m.env ?? {}).forEach(scan);
  Object.values(m.headers ?? {}).forEach(scan);
  return found;
}
