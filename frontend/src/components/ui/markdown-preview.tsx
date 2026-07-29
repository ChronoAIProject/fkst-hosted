import { createElement, type ReactNode } from 'react';

const INLINE_TOKEN_RE =
  /(`[^`\n]+`|\*\*[^*\n]+\*\*|__[^_\n]+__|\[[^\n]+?\]\([^\s)]+\)|\*[^*\n]+\*|_[^_\n]+_)/g;
const FENCE_RE = /^\s*```([^`]*)\s*$/;
const HEADING_RE = /^(#{1,6})\s+(.+)$/;
const UNORDERED_ITEM_RE = /^\s*[-+*]\s+(.+)$/;
const ORDERED_ITEM_RE = /^\s*\d+\.\s+(.+)$/;
/** A pipe table row: at least one `|`, and the line starts or ends with one. */
const TABLE_ROW_RE = /^\s*\|.*\|\s*$/;
/** The delimiter row under a table header (`|---|:--:|`). It is what distinguishes a
 *  real table from a paragraph that happens to contain pipes. */
const TABLE_DIVIDER_RE = /^\s*\|(\s*:?-{1,}:?\s*\|)+\s*$/;

/** Split one pipe-table row into its cells, dropping the leading/trailing empties the
 *  outer pipes produce. */
function tableCells(line: string): string[] {
  return line
    .trim()
    .replace(/^\|/, '')
    .replace(/\|$/, '')
    .split('|')
    .map((cell) => cell.trim());
}

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
    ORDERED_ITEM_RE.test(line) ||
    // A table row must end a paragraph too, or the header line is swallowed into the
    // prose above it and the table never starts.
    TABLE_ROW_RE.test(line)
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

    // A table needs its delimiter row to be a table at all; without it the pipes are
    // ordinary text and fall through to the paragraph branch.
    if (
      TABLE_ROW_RE.test(line) &&
      lineIndex + 1 < lines.length &&
      TABLE_DIVIDER_RE.test(lines[lineIndex + 1]!)
    ) {
      const header = tableCells(line);
      lineIndex += 2;
      const rows: string[][] = [];
      while (lineIndex < lines.length && TABLE_ROW_RE.test(lines[lineIndex]!)) {
        rows.push(tableCells(lines[lineIndex]!));
        lineIndex += 1;
      }
      nodes.push(
        <div key={`block-${blockIndex}`} className="overflow-x-auto">
          <table className="w-full border-collapse text-left">
            <thead>
              <tr>
                {header.map((cell, cellIndex) => (
                  <th
                    key={cellIndex}
                    className="border-b border-line px-2 py-1 font-mono text-[10px] font-semibold uppercase tracking-[0.1em] text-ghost"
                  >
                    {inlineNodes(cell, `block-${blockIndex}-h-${cellIndex}`)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((row, rowIndex) => (
                <tr key={rowIndex}>
                  {/* Indexed by the HEADER width, so a short or long row cannot shift
                      the columns out of alignment. */}
                  {header.map((_, cellIndex) => (
                    <td
                      key={cellIndex}
                      className="border-b border-line-2 px-2 py-1 align-top text-faint"
                    >
                      {inlineNodes(row[cellIndex] ?? '', `block-${blockIndex}-${rowIndex}-${cellIndex}`)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
      blockIndex += 1;
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

/** How the rendered markdown is framed.
 *
 *  `boxed` is the original: a fixed-height, self-scrolling panel — right for a
 *  PREVIEW of something (an issue body beside a form), where the surrounding page owns
 *  the layout and the preview must not push it around.
 *
 *  `flow` renders the markdown as ordinary content with no box and no height cap —
 *  right when the markdown IS the content. In the chat transcript the boxed variant
 *  put a 256px scroll area inside a message inside the already-scrolling transcript:
 *  answers appeared truncated mid-sentence while the panel below them sat empty, and
 *  short answers still reserved 132px of blank space. */
export type MarkdownPreviewVariant = 'boxed' | 'flow';

const VARIANT_CLASSES: Record<MarkdownPreviewVariant, string> = {
  boxed:
    'min-h-[132px] max-h-64 overflow-auto rounded-control border border-line bg-glass px-3 py-2.5 leading-5',
  // No overflow rule of its own: a long answer grows the message and the transcript
  // scrolls, which is the one scrollbar the reader expects.
  flow: 'leading-[1.65]',
};

/** Small, safe GitHub-issue preview. Markdown is converted only to React
 * elements; raw HTML remains text and links are restricted to HTTP(S). */
export function MarkdownPreview({
  markdown,
  ariaLabel,
  variant = 'boxed',
}: {
  markdown: string;
  ariaLabel: string;
  variant?: MarkdownPreviewVariant;
}) {
  return (
    <div
      role="region"
      aria-label={ariaLabel}
      className={`font-mono text-[12px] text-dim space-y-2 ${VARIANT_CLASSES[variant]}`}
    >
      {markdownNodes(markdown)}
    </div>
  );
}
