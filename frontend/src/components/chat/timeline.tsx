import { useState } from 'react';
import { Chip } from '@/components/ui/chip';
import { useContent } from '@/i18n';
import { formatBytes, formatPayload, toolSteps } from './steps';
import type { ChatRoundStep, ChatStep, ChatToolStep, ChatViewLevel } from './steps';

/**
 * Tools whose 404 means the thing is ABSENT rather than hidden.
 *
 * The distinction is per-endpoint and cannot be read off the status alone.
 * `observe_session` deliberately answers 404 for "no runtime, OR you cannot see it" —
 * collapsing those is what stops it leaking whether a session exists — so a 404 there
 * really is the same class of answer as a 403. The log and environment reads decide
 * access separately (they return 403), so their 404 is unambiguous: no logs yet, no
 * such profile.
 *
 * Anything not listed keeps the conservative reading, because mislabelling a hidden
 * thing as absent is the worse error of the two.
 */
const ABSENT_ON_404 = new Set([
  'list_log_runs',
  'get_log_manifest',
  'tail_log_file',
  'get_environment_profile',
]);

/** How a tool step reads. The TEXT always carries the meaning — a chip's colour
 *  is reinforcement, never the signal. */
function toolState(step: ChatToolStep, s: ReturnType<typeof useContent>['chat']) {
  if (step.status == null) {
    return { text: s.toolRunning, tone: 'neutral' as const, running: true };
  }
  if (step.status >= 200 && step.status < 300) {
    return { text: `${s.toolOk} ${step.status}`, tone: 'green' as const, running: false };
  }
  // 409 is "nothing to observe here" on every endpoint that returns it, so it needs
  // no per-tool judgement.
  if (step.status === 409 || (step.status === 404 && ABSENT_ON_404.has(step.name))) {
    // "There is nothing there" is not "you may not see it". Labelling a session's
    // missing logs DENIED told users they lacked access, and it reads neutral rather
    // than red because an absent thing is a fact, not a fault.
    return { text: `${s.toolNone} ${step.status}`, tone: 'neutral' as const, running: false };
  }
  if (step.status === 401 || step.status === 403 || step.status === 404) {
    // A denial is an answer about the USER's access, not a fault — worth saying
    // differently from a genuine failure.
    return { text: `${s.toolDenied} ${step.status}`, tone: 'red' as const, running: false };
  }
  return { text: `${s.toolError} ${step.status}`, tone: 'red' as const, running: false };
}

/** One labelled block of the expanded detail. `overflow-x-auto` on the <pre> is
 *  load-bearing: a long JSON line must scroll inside its own box rather than
 *  widening the panel. */
function DetailBlock({
  label,
  body,
  truncated,
  truncatedNote,
}: {
  label: string;
  body: string;
  truncated?: boolean;
  truncatedNote: string;
}) {
  return (
    <div className="flex flex-col gap-1">
      <span className="font-mono text-[9.5px] uppercase tracking-[0.14em] text-ghost">
        {label}
        {truncated ? ` · ${truncatedNote}` : ''}
      </span>
      <pre className="max-h-64 overflow-auto rounded-control bg-[color-mix(in_oklab,var(--fg)_4%,transparent)] p-2 font-mono text-[10.5px] leading-relaxed text-faint">
        {body || '—'}
      </pre>
    </div>
  );
}

/** A tool step: a summary row that expands to exactly what was sent and returned. */
function ToolRow({ step }: { step: ChatToolStep }) {
  const s = useContent().chat;
  const [expanded, setExpanded] = useState(false);
  const state = toolState(step, s);
  // Humanized where we have a name for it, raw otherwise — a newer backend tool
  // still renders legibly instead of showing nothing.
  const label = s.toolNames[step.name] ?? step.name;
  const size = formatBytes(step.bytes);

  return (
    <li className="flex flex-col">
      <button
        type="button"
        onClick={() => setExpanded((current) => !current)}
        aria-expanded={expanded}
        data-testid="chat-step-tool"
        className="flex w-full items-center gap-2 rounded-control px-1 py-1 text-left transition-colors hover:bg-[color-mix(in_oklab,var(--fg)_4%,transparent)] cursor-pointer"
      >
        <span aria-hidden="true" className="font-mono text-[10px] text-ghost">
          {expanded ? '▾' : '▸'}
        </span>
        <span className="flex-1 truncate font-mono text-[10.5px] text-faint">{label}</span>
        {size && <span className="font-mono text-[10px] text-ghost">{size}</span>}
        <Chip tone={state.tone}>
          {state.running && (
            <span aria-hidden="true" className="anim-dot-blink mr-1 inline-block">
              ·
            </span>
          )}
          {state.text}
          {step.truncated ? ` ${s.toolTruncated}` : ''}
        </Chip>
      </button>
      {expanded && (
        <div className="ml-4 flex flex-col gap-2 border-l border-[color-mix(in_oklab,var(--fg)_10%,transparent)] py-2 pl-3">
          <DetailBlock
            label={s.stepParameters}
            body={formatPayload(step.args ?? step.argsPreview)}
            truncated={step.argsTruncated}
            truncatedNote={s.stepTruncated}
          />
          <DetailBlock
            label={s.stepResponse}
            body={formatPayload(step.response)}
            truncated={step.responseTruncated}
            truncatedNote={s.stepTruncated}
          />
        </div>
      )}
    </li>
  );
}

/** A round marker plus what the model said in that round.
 *
 *  A round with no prose renders an explicit "said nothing" line rather than
 *  nothing at all: "the model went straight to calling tools" is information, and
 *  an empty gap would read as a rendering bug. */
function RoundRow({ step }: { step: ChatRoundStep }) {
  const s = useContent().chat;
  const open = step.finishReason == null;
  const said = step.text?.trim();
  return (
    <li data-testid="chat-step-round" className="flex flex-col gap-1 pt-1">
      <div className="flex items-center gap-2 px-1 font-mono text-[10px] uppercase tracking-[0.12em] text-ghost">
        <span aria-hidden="true">{open ? '◇' : '◆'}</span>
        <span>
          {s.stepRound} {step.index + 1}
        </span>
        <span className="h-px flex-1 bg-[color-mix(in_oklab,var(--fg)_10%,transparent)]" />
        <span>
          {open
            ? s.stepRoundOpen
            : `${step.finishReason} · ${step.toolCalls ?? 0} ${s.stepRoundCalls}`}
        </span>
      </div>
      {said ? (
        <p
          data-testid="chat-step-round-text"
          className="ml-4 border-l border-[color-mix(in_oklab,var(--fg)_10%,transparent)] py-0.5 pl-3 text-[11.5px] leading-relaxed text-faint"
        >
          {said}
        </p>
      ) : (
        !open && (
          <p
            data-testid="chat-step-round-silent"
            className="ml-4 border-l border-[color-mix(in_oklab,var(--fg)_10%,transparent)] py-0.5 pl-3 font-mono text-[10px] italic text-ghost"
          >
            {s.stepRoundSilent}
          </p>
        )
      )}
    </li>
  );
}

/**
 * The orchestration timeline for one assistant message.
 *
 * VERBOSE renders the whole loop — every round boundary and every tool call, each
 * on its own line and each expandable to the exact parameters and response. CLEAN
 * collapses it to a single count, because most of the time the machinery is noise
 * and the answer is the point.
 *
 * `level` is the level captured when THIS turn started, not the current setting:
 * toggling must not rewrite turns already on screen.
 */
export function Timeline({ steps, level }: { steps: ChatStep[]; level: ChatViewLevel }) {
  const s = useContent().chat;
  if (steps.length === 0) return null;

  const tools = toolSteps(steps);

  if (level === 'clean') {
    if (tools.length === 0) return null;
    return (
      <p data-testid="chat-timeline-summary" className="font-mono text-[10px] text-ghost">
        {tools.length} {tools.length === 1 ? s.stepSummaryOne : s.stepSummaryMany}
      </p>
    );
  }

  return (
    <ul data-testid="chat-timeline" className="flex flex-col gap-0.5">
      {steps.map((step) =>
        step.kind === 'round' ? (
          <RoundRow key={`round-${step.index}`} step={step} />
        ) : (
          <ToolRow key={`tool-${step.id}`} step={step} />
        )
      )}
    </ul>
  );
}
