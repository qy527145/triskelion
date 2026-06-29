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
