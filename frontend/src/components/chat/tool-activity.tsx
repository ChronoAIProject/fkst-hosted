import { useState } from 'react';
import { Chip } from '@/components/ui/chip';
import { useContent } from '@/i18n';
import type { ChatToolEvent } from './chat-context';

/** Beyond this many rows the list collapses behind a disclosure: showing the
 *  machine's work is a trust feature, burying the answer under it is not. */
const COLLAPSE_ABOVE = 3;

/** How a tool event reads. The TEXT always carries the meaning — a chip's colour
 *  is reinforcement, never the signal. */
function toolState(event: ChatToolEvent, s: ReturnType<typeof useContent>['chat']) {
  if (event.status == null) {
    return { text: s.toolRunning, tone: 'neutral' as const, running: true };
  }
  if (event.status >= 200 && event.status < 300) {
    return { text: `${s.toolOk} ${event.status}`, tone: 'green' as const, running: false };
  }
  if (event.status === 401 || event.status === 403 || event.status === 404) {
    // A denial is an answer about the USER's access, not a fault — worth saying
    // differently from a genuine failure.
    return { text: `${s.toolDenied} ${event.status}`, tone: 'red' as const, running: false };
  }
  return { text: `${s.toolError} ${event.status}`, tone: 'red' as const, running: false };
}

function ToolRow({ event }: { event: ChatToolEvent }) {
  const s = useContent().chat;
  const state = toolState(event, s);
  // Humanized where we have a name for it, raw otherwise — a newer backend tool
  // still renders legibly instead of showing nothing.
  const label = s.toolNames[event.name] ?? event.name;
  return (
    <span className="inline-flex items-center gap-1">
      <span aria-hidden="true" className="font-mono text-[10px] text-ghost">
        ▸
      </span>
      <span className="font-mono text-[10.5px] text-faint">{label}</span>
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
}

/**
 * Live tool activity for one assistant message — "the machine shows its work".
 *
 * It is a trust feature as much as a HUD beat: a user who can see that the answer
 * came from `tail_log_file` returning 403 knows exactly why it says what it says.
 */
export function ToolActivity({ events }: { events: ChatToolEvent[] }) {
  const s = useContent().chat;
  const [expanded, setExpanded] = useState(false);

  if (events.length === 0) return null;

  const collapsible = events.length > COLLAPSE_ABOVE;
  const shown = collapsible && !expanded ? events.slice(0, COLLAPSE_ABOVE) : events;

  return (
    <div data-testid="chat-tool-events" className="flex flex-col gap-1">
      <div className="flex flex-wrap items-center gap-1.5">
        {shown.map((event) => (
          <ToolRow key={event.id} event={event} />
        ))}
      </div>
      {collapsible && (
        <button
          type="button"
          onClick={() => setExpanded((current) => !current)}
          aria-expanded={expanded}
          data-testid="chat-activity-toggle"
          className="self-start rounded-control font-mono text-[10px] uppercase tracking-[0.12em] text-ghost transition-colors hover:text-faint cursor-pointer"
        >
          {s.activityToggle} {expanded ? '▴' : '▾'} ({events.length})
        </button>
      )}
    </div>
  );
}
