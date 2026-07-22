import { createElement, type ReactNode } from 'react';

const INLINE_TOKEN_RE =
  /(`[^`\n]+`|\*\*[^*\n]+\*\*|__[^_\n]+__|\[[^\n]+?\]\([^\s)]+\)|\*[^*\n]+\*|_[^_\n]+_)/g;
const FENCE_RE = /^\s*```([^`]*)\s*$/;
const HEADING_RE = /^(#{1,6})\s+(.+)$/;
const UNORDERED_ITEM_RE = /^\s*[-+*]\s+(.+)$/;
const ORDERED_ITEM_RE = /^\s*\d+\.\s+(.+)$/;

const HEADING_CLASSES = [
  'text-[18px]',
  'text-[16px]',
  'text-[14px]',
  'text-[13px]',
  'text-[12px]',
  'text-[11px]',
] as const;

function safeLink(href: string): boolean {
  try {
    const url = new URL(href);
    return url.protocol === 'https:' || url.protocol === 'http:';
  } catch {
    return false;
  }
}

function inlineNodes(text: string, keyPrefix: string): ReactNode[] {
  return text.split(INLINE_TOKEN_RE).flatMap((part, index) => {
    if (!part) return [];
    const key = `${keyPrefix}-${index}`;
    if (part.startsWith('`') && part.endsWith('`')) {
      return (
        <code key={key} className="rounded-control bg-glass-2 px-1 py-0.5 text-amber">
          {part.slice(1, -1)}
        </code>
      );
    }
    if (
      (part.startsWith('**') && part.endsWith('**')) ||
      (part.startsWith('__') && part.endsWith('__'))
    ) {
      return <strong key={key}>{inlineNodes(part.slice(2, -2), `${key}-strong`)}</strong>;
    }
    if (part.startsWith('[')) {
      const link = /^\[([^\n]+)\]\(([^)\s]+)\)$/.exec(part);
      if (link && safeLink(link[2]!)) {
        return (
          <a
            key={key}
            href={link[2]}
            target="_blank"
            rel="noreferrer noopener"
            className="text-amber underline underline-offset-2 break-all"
          >
            {inlineNodes(link[1]!, `${key}-link`)}
          </a>
        );
      }
      return part;
    }
    if (
      (part.startsWith('*') && part.endsWith('*')) ||
      (part.startsWith('_') && part.endsWith('_'))
    ) {
      return <em key={key}>{inlineNodes(part.slice(1, -1), `${key}-em`)}</em>;
    }
    return part;
  });
}

function startsBlock(line: string): boolean {
  return (
    FENCE_RE.test(line) ||
    HEADING_RE.test(line) ||
    UNORDERED_ITEM_RE.test(line) ||
    ORDERED_ITEM_RE.test(line)
  );
}

function markdownNodes(markdown: string): ReactNode[] {
  const lines = markdown.replace(/\r\n?/g, '\n').split('\n');
  const nodes: ReactNode[] = [];
  let lineIndex = 0;
  let blockIndex = 0;

  while (lineIndex < lines.length) {
    const line = lines[lineIndex]!;
    if (line.trim() === '') {
      lineIndex += 1;
      continue;
    }

    const fence = FENCE_RE.exec(line);
    if (fence) {
      const language = fence[1]!.trim();
      const code: string[] = [];
      lineIndex += 1;
      while (lineIndex < lines.length && !/^\s*```\s*$/.test(lines[lineIndex]!)) {
        code.push(lines[lineIndex]!);
        lineIndex += 1;
      }
      if (lineIndex < lines.length) lineIndex += 1;
      nodes.push(
        <pre
          key={`block-${blockIndex}`}
          className="overflow-x-auto rounded-control border border-line bg-glass-2 p-3 font-mono text-[11px] text-fg whitespace-pre"
        >
          <code data-language={language || undefined}>{code.join('\n')}</code>
        </pre>
      );
      blockIndex += 1;
      continue;
    }

    const heading = HEADING_RE.exec(line);
    if (heading) {
      const level = heading[1]!.length;
      nodes.push(
        createElement(
          `h${level}`,
          {
            key: `block-${blockIndex}`,
            className: `font-ui font-semibold text-fg ${HEADING_CLASSES[level - 1]}`,
          },
          inlineNodes(heading[2]!, `block-${blockIndex}-heading`)
        )
      );
      blockIndex += 1;
      lineIndex += 1;
      continue;
    }

    const unordered = UNORDERED_ITEM_RE.exec(line);
    const ordered = ORDERED_ITEM_RE.exec(line);
    if (unordered || ordered) {
      const orderedList = ordered != null;
      const matcher = orderedList ? ORDERED_ITEM_RE : UNORDERED_ITEM_RE;
      const items: string[] = [];
      while (lineIndex < lines.length) {
        const item = matcher.exec(lines[lineIndex]!);
        if (!item) break;
        items.push(item[1]!);
        lineIndex += 1;
      }
      const listItems = items.map((item, itemIndex) => (
        <li key={`block-${blockIndex}-item-${itemIndex}`}>
          {inlineNodes(item, `block-${blockIndex}-item-${itemIndex}`)}
        </li>
      ));
      nodes.push(
        createElement(
          orderedList ? 'ol' : 'ul',
          {
            key: `block-${blockIndex}`,
            className: `${orderedList ? 'list-decimal' : 'list-disc'} pl-5 space-y-1`,
          },
          listItems
        )
      );
      blockIndex += 1;
      continue;
    }

    const paragraph: string[] = [line];
    lineIndex += 1;
    while (
      lineIndex < lines.length &&
      lines[lineIndex]!.trim() !== '' &&
      !startsBlock(lines[lineIndex]!)
    ) {
      paragraph.push(lines[lineIndex]!);
      lineIndex += 1;
    }
    nodes.push(
      <p key={`block-${blockIndex}`} className="whitespace-pre-wrap break-words">
        {inlineNodes(paragraph.join('\n'), `block-${blockIndex}-paragraph`)}
      </p>
    );
    blockIndex += 1;
  }

  return nodes;
}

/** Small, safe GitHub-issue preview. Markdown is converted only to React
 * elements; raw HTML remains text and links are restricted to HTTP(S). */
export function MarkdownPreview({ markdown, ariaLabel }: { markdown: string; ariaLabel: string }) {
  return (
    <div
      role="region"
      aria-label={ariaLabel}
      className="min-h-[132px] max-h-64 overflow-auto rounded-control border border-line bg-glass px-3 py-2.5 font-mono text-[12px] leading-5 text-dim space-y-2"
    >
      {markdownNodes(markdown)}
    </div>
  );
}
