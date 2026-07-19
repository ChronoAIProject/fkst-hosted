import React from 'react';

// Minimal inline markup for translatable prose:
//   `code`    -> monospace chip (GitHub identifiers, commands, regex)
//   **bold**  -> emphasized foreground text
//   *italic*  -> emphasis
// Everything else renders verbatim, so translators work in plain strings and
// keep the backticked literals untouched.
const TOKEN = /(`[^`]+`|\*\*[^*]+\*\*|\*[^*]+\*)/g;

export function Rich({ children }: { children: string }) {
  const parts = children.split(TOKEN);
  return (
    <>
      {parts.map((part, i) => {
        if (part.length >= 2 && part.startsWith('`') && part.endsWith('`')) {
          return (
            <code
              key={i}
              // Refined inline chip: raised surface + a hairline for a crisper,
              // more premium code literal within prose.
              className="font-mono text-[0.92em] text-fg bg-raise-2 border border-line rounded-chip px-1 py-0.5"
            >
              {part.slice(1, -1)}
            </code>
          );
        }
        if (part.length >= 4 && part.startsWith('**') && part.endsWith('**')) {
          return (
            <span key={i} className="text-fg font-medium">
              {part.slice(2, -2)}
            </span>
          );
        }
        if (part.length >= 2 && part.startsWith('*') && part.endsWith('*')) {
          return (
            <em key={i} className="text-dim not-italic">
              {part.slice(1, -1)}
            </em>
          );
        }
        return <React.Fragment key={i}>{part}</React.Fragment>;
      })}
    </>
  );
}
