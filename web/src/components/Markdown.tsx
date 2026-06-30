import { Fragment, type ReactNode } from "react";

/**
 * 极简 Markdown 渲染器（无第三方依赖）。
 * 支持：# 标题、``` 代码块、- / * 列表、> 引用、`行内代码`、**粗体**、[文本](链接)。
 * 仅产出 React 元素，不注入裸 HTML，避免 XSS。
 */
export default function Markdown({ text }: { text: string }) {
  return <div className="text-sm leading-7 text-slate-700">{render(text)}</div>;
}

function render(text: string): ReactNode[] {
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  const out: ReactNode[] = [];
  let i = 0;
  let key = 0;
  const k = () => key++;

  while (i < lines.length) {
    const line = lines[i];

    // 代码块
    if (line.trim().startsWith("```")) {
      const buf: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trim().startsWith("```")) {
        buf.push(lines[i]);
        i++;
      }
      i++; // 跳过结束 ```
      out.push(
        <pre
          key={k()}
          className="my-2.5 overflow-auto rounded-xl border border-slate-200 bg-slate-50 p-3.5 font-mono text-xs leading-relaxed text-slate-700"
        >
          {buf.join("\n")}
        </pre>,
      );
      continue;
    }

    // 标题
    const h = line.match(/^(#{1,4})\s+(.*)$/);
    if (h) {
      const level = h[1].length;
      const cls =
        level === 1
          ? "mt-4 mb-2 text-lg font-bold text-slate-800"
          : level === 2
            ? "mt-3.5 mb-1.5 text-base font-bold text-slate-800"
            : "mt-3 mb-1 text-sm font-semibold text-slate-700";
      out.push(
        <div key={k()} className={cls}>
          {inline(h[2])}
        </div>,
      );
      i++;
      continue;
    }

    // 列表
    if (/^\s*[-*]\s+/.test(line)) {
      const items: ReactNode[] = [];
      while (i < lines.length && /^\s*[-*]\s+/.test(lines[i])) {
        items.push(
          <li key={k()} className="ml-5 list-disc">
            {inline(lines[i].replace(/^\s*[-*]\s+/, ""))}
          </li>,
        );
        i++;
      }
      out.push(
        <ul key={k()} className="my-2 space-y-1">
          {items}
        </ul>,
      );
      continue;
    }

    // 引用
    if (line.trim().startsWith(">")) {
      out.push(
        <blockquote
          key={k()}
          className="my-2 border-l-4 border-indigo-200 bg-indigo-50/50 px-3 py-1.5 text-slate-600"
        >
          {inline(line.replace(/^\s*>\s?/, ""))}
        </blockquote>,
      );
      i++;
      continue;
    }

    // 空行
    if (line.trim() === "") {
      i++;
      continue;
    }

    // 段落（连续非空行合并）
    const para: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() !== "" &&
      !lines[i].trim().startsWith("```") &&
      !/^(#{1,4})\s+/.test(lines[i]) &&
      !/^\s*[-*]\s+/.test(lines[i]) &&
      !lines[i].trim().startsWith(">")
    ) {
      para.push(lines[i]);
      i++;
    }
    out.push(
      <p key={k()} className="my-2">
        {inline(para.join(" "))}
      </p>,
    );
  }
  return out;
}

/** 行内：`code`、**bold**、[text](url)。 */
function inline(text: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  // 先按行内代码切分，代码段内不再解析其他语法。
  const parts = text.split(/(`[^`]+`)/g);
  let key = 0;
  for (const part of parts) {
    if (part.startsWith("`") && part.endsWith("`") && part.length >= 2) {
      nodes.push(
        <code
          key={key++}
          className="rounded bg-slate-100 px-1.5 py-0.5 font-mono text-[0.8em] text-indigo-600"
        >
          {part.slice(1, -1)}
        </code>,
      );
    } else {
      nodes.push(<Fragment key={key++}>{boldAndLinks(part)}</Fragment>);
    }
  }
  return nodes;
}

function boldAndLinks(text: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  let key = 0;
  // **bold**
  const segs = text.split(/(\*\*[^*]+\*\*)/g);
  for (const seg of segs) {
    if (seg.startsWith("**") && seg.endsWith("**") && seg.length >= 4) {
      nodes.push(
        <strong key={key++} className="font-semibold text-slate-800">
          {seg.slice(2, -2)}
        </strong>,
      );
    } else {
      nodes.push(<Fragment key={key++}>{links(seg)}</Fragment>);
    }
  }
  return nodes;
}

function links(text: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  const re = /\[([^\]]+)\]\(([^)]+)\)/g;
  let last = 0;
  let m: RegExpExecArray | null;
  let key = 0;
  while ((m = re.exec(text))) {
    if (m.index > last) nodes.push(text.slice(last, m.index));
    nodes.push(
      <a
        key={key++}
        href={m[2]}
        target="_blank"
        rel="noreferrer"
        className="text-indigo-500 underline decoration-indigo-300 hover:text-indigo-600"
      >
        {m[1]}
      </a>,
    );
    last = re.lastIndex;
  }
  if (last < text.length) nodes.push(text.slice(last));
  return nodes;
}
