import { useCallback, useEffect, useRef, useState } from 'react';
import { ScrollArea } from '@/components/ui/scroll-area';
import { StaggerItem } from '@/components/ui/motion';
import { useContent } from '@/i18n';
import { useChat } from './chat-context';
import { MessageBubble } from './message-bubble';

/** How close to the bottom still counts as "following the conversation". Loose
 *  enough to survive sub-pixel rounding, tight enough that a deliberate scroll-up
 *  is respected immediately. */
const STICK_THRESHOLD_PX = 40;

/** The corner brackets that frame a HUD surface. Four 1px L-shapes, pure CSS and
 *  fully static, so they need no reduced-motion entry. */
function CornerBrackets() {
  const accent = 'color-mix(in oklab, var(--amber) 55%, transparent)';
  return (
    <>
      <span
        aria-hidden="true"
        className="pointer-events-none absolute left-0 top-0 h-2 w-2 border-l border-t"
        style={{ borderColor: accent }}
      />
      <span
        aria-hidden="true"
        className="pointer-events-none absolute right-0 top-0 h-2 w-2 border-r border-t"
        style={{ borderColor: accent }}
      />
      <span
        aria-hidden="true"
        className="pointer-events-none absolute bottom-0 left-0 h-2 w-2 border-b border-l"
        style={{ borderColor: accent }}
      />
      <span
        aria-hidden="true"
        className="pointer-events-none absolute bottom-0 right-0 h-2 w-2 border-b border-r"
        style={{ borderColor: accent }}
      />
    </>
  );
}

/** The empty state: what the Orchestrator is for, plus three prompts that fill the
 *  composer so a first-time user never has to invent an opening line. */
function WelcomeCard({ onPick }: { onPick: (text: string) => void }) {
  const s = useContent().chat;
  const starters = [s.starters.running, s.starters.unrouted, s.starters.start];
  return (
    <div className="relative rounded-card border border-line bg-glass backdrop-blur-glass p-4 shadow-[var(--shadow-1),var(--highlight-top)]">
      <CornerBrackets />
      <p className="font-display text-[14px] font-semibold text-fg">{s.welcomeTitle}</p>
      <p className="mt-1.5 text-[12.5px] leading-relaxed text-dim">{s.welcomeBody}</p>
      <div className="mt-3 flex flex-wrap gap-1.5">
        {starters.map((starter) => (
          <button
            key={starter}
            type="button"
            onClick={() => onPick(starter)}
            className="rounded-chip border border-line-2 bg-raise-2 px-2 py-1 font-mono text-[10.5px] text-faint transition-colors hover:border-[color-mix(in_oklab,var(--amber)_40%,var(--line-2))] hover:text-fg cursor-pointer"
          >
            {starter}
          </button>
        ))}
      </div>
    </div>
  );
}

/**
 * The transcript. Owns its own scrolling (the shell's viewport never scrolls) and
 * the stick-to-bottom rule.
 *
 * Stick-to-bottom reads `scrollTop` on the element, not `window.scrollY`, because
 * in this fixed-viewport shell `window.scrollY` is always 0 — every scroll happens
 * inside a panel like this one. A user who scrolls up to read something is NOT
 * dragged back down; they get a "jump to latest" affordance instead.
 */
export function MessageList({ onPickStarter }: { onPickStarter: (text: string) => void }) {
  const s = useContent().chat;
  const { messages } = useChat();
  const scrollRef = useRef<HTMLDivElement>(null);
  const [detached, setDetached] = useState(false);

  const atBottom = useCallback(() => {
    const element = scrollRef.current;
    if (element == null) return true;
    return element.scrollHeight - element.scrollTop - element.clientHeight <= STICK_THRESHOLD_PX;
  }, []);

  const scrollToBottom = useCallback(() => {
    const element = scrollRef.current;
    if (element == null) return;
    element.scrollTop = element.scrollHeight;
    setDetached(false);
  }, []);

  // The listener goes on the SCROLLING element via the forwarded ref, not on a
  // child through React's `onScroll`: native scroll events do not bubble, so a
  // handler on the inner list would never fire.
  useEffect(() => {
    const element = scrollRef.current;
    if (element == null) return;
    const onScroll = () => setDetached(!atBottom());
    element.addEventListener('scroll', onScroll, { passive: true });
    return () => element.removeEventListener('scroll', onScroll);
  }, [atBottom]);

  // Follow new content only while the user is still at the bottom. The dependency
  // is the whole message list so a growing assistant answer keeps following.
  useEffect(() => {
    if (!detached) scrollToBottom();
    // `detached` is read, not depended on: including it would re-scroll the moment
    // a user scrolls back down, which the scroll listener already handles.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [messages, scrollToBottom]);

  return (
    <div className="relative flex flex-1 flex-col min-h-0">
      <ScrollArea ref={scrollRef} className="px-3 py-3">
        <div
          role="log"
          aria-live="polite"
          aria-label={s.transcriptAria}
          data-testid="chat-transcript"
          className="flex flex-col gap-3"
        >
          {messages.length === 0 ? (
            <WelcomeCard onPick={onPickStarter} />
          ) : (
            messages.map((message, index) => (
              <StaggerItem key={message.id} index={index}>
                <MessageBubble message={message} />
              </StaggerItem>
            ))
          )}
        </div>
      </ScrollArea>

      {detached && (
        <button
          type="button"
          onClick={scrollToBottom}
          data-testid="chat-jump-latest"
          className="absolute bottom-3 left-1/2 -translate-x-1/2 rounded-pill border border-line-2 bg-raise-2 px-2.5 py-1 font-mono text-[10px] font-semibold uppercase tracking-[0.12em] text-faint shadow-2 transition-colors hover:text-fg cursor-pointer"
        >
          {s.jumpToLatest} ↓
        </button>
      )}
    </div>
  );
}
