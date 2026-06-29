const PALETTE = [
  "#6d5ef0",
  "#3b82f6",
  "#0ea5a4",
  "#ef6c45",
  "#8b5cf6",
  "#e0457b",
  "#f59e0b",
  "#22a06b",
];

/** 由字符串稳定派生一个主题色，用于卡片图标渐变。 */
export function colorFor(s: string): string {
  let h = 0;
  for (const c of s) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return PALETTE[h % PALETTE.length];
}

/** 取名称首两位字母/数字作为图标文字。 */
export function initials(s: string): string {
  return (s || "?").replace(/[^a-zA-Z0-9]/g, "").slice(0, 2).toUpperCase() || "MC";
}
