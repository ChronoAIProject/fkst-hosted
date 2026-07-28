import { useEffect, useRef } from 'react';
import { FIELD_INPUT } from '@/components/ui/field';
import { useContent } from '@/i18n';
import { useChat } from './chat-context';

/** Hard cap on one message. Generous for a question, bounded so a pasted log
 *  cannot become the prompt. */
export const MAX_MESSAGE_CHARS = 8000;

/** Show the counter only once it is worth watching. */
const COUNTER_THRESHOLD = 0.9;

/** Auto-grow bounds, in rows. */
const MIN_ROWS = 1;
const MAX_ROWS = 6;
/** Line height used to convert rows to pixels; matches the textarea's leading. */
const LINE_HEIGHT_PX = 20;

/**
 * The input row. Enter sends, Shift+Enter adds a newline — the convention a chat
 * user already has, and the reason this is a textarea rather than an input.
 *
 * While a turn is streaming the send button becomes Stop, so the primary action is
 * always the one the user wants next and the button never sits uselessly disabled.
 */
export function Composer({ value, onChange }: { value: string; onChange: (next: string) => void }) {
  const s = useContent().chat;
  const { sendMessage, stopStreaming, streaming } = useChat();
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Auto-grow: reset then measure, because `scrollHeight` on an unshrunk textarea
  // only ever reports the taller of the two.
  useEffect(() => {
    const element = textareaRef.current;
    if (element == null) return;
    element.style.height = 'auto';
    const max = MAX_ROWS * LINE_HEIGHT_PX + 16;
    const min = MIN_ROWS * LINE_HEIGHT_PX + 16;
    element.style.height = `${Math.min(Math.max(element.scrollHeight, min), max)}px`;
  }, [value]);

  const trimmed = value.trim();
  const canSend = trimmed.length > 0 && !streaming;

  const submit = () => {
    if (!canSend) return;
    sendMessage(trimmed);
    onChange('');
  };

  const showCounter = value.length >= MAX_MESSAGE_CHARS * COUNTER_THRESHOLD;

  return (
    <div className="flex-none border-t border-line px-3 py-2.5">
      <div className="flex items-end gap-2">
        <textarea
          ref={textareaRef}
          value={value}
          onChange={(event) => onChange(event.target.value.slice(0, MAX_MESSAGE_CHARS))}
          onKeyDown={(event) => {
            if (event.key !== 'Enter' || event.shiftKey) return;
            // Enter sends; Shift+Enter falls through to the default newline.
            event.preventDefault();
            submit();
          }}
          rows={MIN_ROWS}
          aria-label={s.inputAria}
          placeholder={s.placeholder}
          data-testid="chat-input"
          className={`${FIELD_INPUT} resize-none font-mono text-[12.5px] leading-5`}
        />
        {streaming ? (
          <button
            type="button"
            onClick={stopStreaming}
            aria-label={s.stopAria}
            data-testid="chat-stop"
            // Secondary recipe with --warn text: stopping is a caution, and the
            // brand accent must never stand in for a warning.
            className="glass grad-border flex-none rounded-control px-3 py-2 font-ui text-[12.5px] font-semibold text-warn transition-colors hover:bg-raise-2 cursor-pointer"
          >
            {s.stop}
          </button>
        ) : (
          <button
            type="button"
            onClick={submit}
            disabled={!canSend}
            aria-label={s.sendAria}
            data-testid="chat-send"
            className="flex-none rounded-control bg-grad-accent px-3 py-2 font-ui text-[12.5px] font-semibold text-amber-ink shadow-[var(--shadow-2),var(--glow-amber)] transition-[filter,opacity] hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-40 cursor-pointer"
          >
            {s.send}
          </button>
        )}
      </div>
      {showCounter && (
        <p
          data-testid="chat-char-count"
          className="mt-1 text-right font-mono text-[10px] text-ghost"
        >
          {s.charCount
            .replace('{used}', String(value.length))
            .replace('{max}', String(MAX_MESSAGE_CHARS))}
        </p>
      )}
    </div>
  );
}
