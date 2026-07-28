import { Chip } from '@/components/ui/chip';
import { CopyButton } from '@/components/ui/copy-button';
import { MarkdownPreview } from '@/components/ui/markdown-preview';
import { useContent } from '@/i18n';
import type { ChatMessage, ChatToolEvent } from './chat-context';

/** How a tool event reads: state text plus a tone. The TEXT always carries the
 *  meaning — a chip's colour is reinforcement, never the signal. */
function toolState(event: ChatToolEvent, s: ReturnType<typeof useContent>['chat']) {
  if (event.status == null) return { text: s.toolRunning, tone: 'neutral' as const, running: true };
  if (event.status >= 200 && event.status < 300) {
    return { text: `${s.toolOk} ${event.status}`, tone: 'green' as const, running: false };
  }
  if (event.status === 401 || event.status === 403 || event.status === 404) {
    // A denial is an answer about the user's access, not a fault — worth
    // distinguishing from a genuine failure.
    return { text: `${s.toolDenied} ${event.status}`, tone: 'red' as const, running: false };
  }
  return { text: `${s.toolError} ${event.status}`, tone: 'red' as const, running: false };
}

/** The compact tool-activity row. A fuller visualization arrives with the real
 *  transport; this already shows the machine working, which is the trust beat. */
function ToolEvents({ events }: { events: ChatToolEvent[] }) {
  const s = useContent().chat;
  if (events.length === 0) return null;
  return (
    <div data-testid="chat-tool-events" className="flex flex-wrap items-center gap-1.5">
      {events.map((event) => {
        const state = toolState(event, s);
        return (
          <span key={event.id} className="inline-flex items-center gap-1">
            <span aria-hidden="true" className="font-mono text-[10px] text-ghost">
              ▸
            </span>
            <span className="font-mono text-[10.5px] text-faint">{event.name}</span>
            <Chip tone={state.tone}>
              {state.running && (
                <span aria-hidden="true" className="anim-dot-blink mr-1 inline-block">
                  ·
                </span>
              )}
              {state.text}
              {event.truncated ? ` ${s.toolTruncated}` : ''}
            </Chip>
          </span>
        );
      })}
    </div>
  );
}

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

      <ToolEvents events={message.toolEvents ?? []} />

      {message.content !== '' && (
        <MarkdownPreview markdown={message.content} ariaLabel={s.answerAria} />
      )}

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
