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
  labels?: string[];
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
export type SkillCategory = "skill" | "kb" | "toolchain" | "agent";

export const SKILL_CATEGORIES: { id: SkillCategory; label: string }[] = [
  { id: "skill", label: "技能" },
  { id: "kb", label: "知识库" },
  { id: "toolchain", label: "工具链" },
  { id: "agent", label: "Agent" },
];

export function categoryLabel(c: string): string {
  return SKILL_CATEGORIES.find((x) => x.id === c)?.label ?? c;
}

/**
 * 该分类对应的「能力说明书」文件名。agent 分类用 AGENT.md，其余用 SKILL.md。
 * 服务端始终以 skill_md 字段承载其全文，文件名仅用于市场展示与本地打包。
 */
export function docFilename(category: string): string {
  return category === "agent" ? "AGENT.md" : "SKILL.md";
}

/** 受管标签的徽章配色：官方=金、社区=天蓝、其余=靛蓝。 */
export function labelBadgeClass(name: string): string {
  if (name === "官方") return "border-amber-200 bg-amber-50 text-amber-700";
  if (name === "社区") return "border-sky-200 bg-sky-50 text-sky-600";
  return "border-indigo-200 bg-indigo-50 text-indigo-600";
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
  labels?: string[];
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

export interface GroupBrief {
  id: number;
  name: string;
}

export interface AdminUser {
  id: number;
  username: string;
  groups: GroupBrief[];
  created_at: string;
  skills: number;
  mcps: number;
  secrets: number;
}

export interface AdminGroup {
  id: number;
  name: string;
  description: string;
  users: number;
  created_at: string;
}

export interface AdminLabel {
  id: number;
  name: string;
  skills: number;
  mcps: number;
  created_at: string;
}

export interface AdminSkill {
  owner: string;
  name: string;
  category: string;
  visibility: string;
  group_visibility: string;
  version: string;
  description: string;
  tags: string[];
  skill_md: string;
  mcp_dependencies: string[];
  preferred_tools: string[];
  archive_size: number;
  has_archive: boolean;
  labels: GroupBrief[];
  updated_at: string;
}

export interface AdminMcp {
  owner: string;
  name: string;
  visibility: string;
  group_visibility: string;
  version: string;
  runtime: string;
  protocol: string;
  manifest: McpManifest;
  labels: GroupBrief[];
  updated_at: string;
}

export interface CallLog {
  caller: string;
  caller_id: number | null;
  owner: string;
  mcp_name: string;
  tool: string;
  ok: boolean;
  error: string;
  result: string;
  ms: number;
  created_at: string;
}

/** 调用日志查询参数（全部可选）。 */
export interface CallsQuery {
  service?: string;
  tool?: string;
  caller?: string;
  /** 时间窗口（小时）；0 / 省略表示不限。 */
  window?: number;
  errors_only?: boolean;
  limit?: number;
  offset?: number;
}

/** 调用日志分页响应：命中总数 + 当前页 + 过滤下拉候选。 */
export interface CallsResp {
  total: number;
  rows: CallLog[];
  services: string[];
  tools: string[];
}

export interface ImportSummary {
  groups: number;
  users: number;
  mcps: number;
  skills: number;
  secrets: number;
  calls: number;
  blobs: number;
  skipped: string[];
}

/** group_visibility 取值：字符串 "all" 或分组 id 数组。 */
export type GroupVisibility = "all" | number[];

/** 把后端存储的 group_visibility 字符串解析为前端模型。 */
export function parseGroupVisibility(raw: string): GroupVisibility {
  const s = (raw ?? "").trim();
  if (s === "" || s === "all") return "all";
  try {
    const arr = JSON.parse(s);
    if (Array.isArray(arr)) return arr.filter((x) => typeof x === "number");
  } catch {
    /* fall through */
  }
  return "all";
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
