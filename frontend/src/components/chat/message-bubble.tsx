import { CopyButton } from '@/components/ui/copy-button';
import { MarkdownPreview } from '@/components/ui/markdown-preview';
import { useContent } from '@/i18n';
import type { ChatMessage } from './chat-context';
import { ActionCards } from './action-card';
import { DataCards } from './data-cards';
import { RichCards } from './rich-cards';
import { Timeline } from './timeline';

/** One transcript entry. Three shapes, because they answer three different
 *  questions: what the assistant said, what the user asked, and what the client
 *  itself needs to report. */
export function MessageBubble({ message }: { message: ChatMessage }) {
  const s = useContent().chat;

  if (message.role === 'system-note') {
    return (
      <div
        data-testid="chat-system-note"
        className={`border-l-2 pl-3 font-mono text-[11.5px] leading-5 ${
          // Warning tone uses --warn, never the brand accent: the accent means
          // "fkst", not "careful".
          message.tone === 'warn' ? 'border-l-warn text-warn' : 'border-l-line text-ghost'
        }`}
      >
        {message.content}
      </div>
    );
  }

  if (message.role === 'user') {
    return (
      <div className="flex justify-end">
        <div
          data-testid="chat-user-message"
          className="max-w-[85%] rounded-card border border-line border-l-2 border-l-[color-mix(in_oklab,var(--amber)_40%,var(--line))] bg-raise px-3 py-2 font-ui text-[13px] leading-relaxed text-fg whitespace-pre-wrap break-words"
        >
          {message.content}
        </div>
      </div>
    );
  }

  return (
    <div
      data-testid="chat-assistant-message"
      // The glass-console card: the same recipe CodeBlock and LogViewer use, so an
      // answer reads as machine output rather than a speech bubble.
      className="rounded-card border border-line bg-glass backdrop-blur-glass shadow-[var(--shadow-2),var(--highlight-top)] p-3 flex flex-col gap-2"
    >
      <div className="flex items-center gap-2">
        <span aria-hidden="true" className="font-mono text-[10px] text-amber">
          ▸
        </span>
        <span className="font-mono text-[10px] font-semibold uppercase tracking-[0.16em] text-faint">
          {s.assistantRole}
        </span>
        <span className="flex-1" aria-hidden="true" />
        {/* Copy the RAW markdown, which is what a user would paste elsewhere. */}
        {message.content !== '' && <CopyButton value={message.content} label={s.copyAnswer} />}
      </div>

      {/* The level captured with THIS message, not the live setting — a toggle
          must not rewrite a turn the user has already read. */}
      <Timeline steps={message.steps ?? []} level={message.viewLevel ?? 'clean'} />

      {/* `flow`, not the boxed preview: the answer IS this message, so it must grow
          with its content and let the TRANSCRIPT scroll. Boxed would nest a second
          scroll area inside a scrolling panel and cut the answer mid-sentence. */}
      {message.content !== '' && (
        <MarkdownPreview markdown={message.content} ariaLabel={s.answerAria} variant="flow" />
      )}

      {/* Structured renderings of the data the turn actually fetched. Above the
          session cards because they answer the question; the session cards are
          navigation. Both are projected server-side from tool results — neither is
          derived from the model's prose. */}
      <DataCards cards={message.dataCards ?? []} />

      {/* Deterministic deep-link cards, derived only from the turn's structured
          tool results — never from the model's prose. */}
      <RichCards refs={message.sessionRefs ?? []} />

      {/* Confirm-gated action cards last, so the answer explaining them is read
          first. */}
      <ActionCards proposals={message.proposals ?? []} />

      {/* The pending caret IS the typing indicator — no separate component, so
          there is never a moment showing both or neither. */}
      {message.pending && (
        <span
          data-testid="chat-pending-caret"
          aria-hidden="true"
          className="anim-dot-blink font-mono text-[13px] leading-none text-amber"
        >
          ▮
        </span>
      )}
    </div>
  );
}
