/**
 * The orchestration timeline: what happened between a question and its answer.
 *
 * Answering one message takes several rounds between the backend and the model —
 * it is offered the tool catalogue, picks a tool, the result is fed back, and it
 * decides the next step. The backend streams that loop as `round_start` /
 * `round_end` / `tool_call` / `tool_result` frames; this module folds them into an
 * ORDERED list, because order is the whole point: a timeline that re-groups its
 * steps is no longer a record of what happened.
 *
 * Kept as plain data with pure reducers so the ordering rules are unit-testable
 * without mounting a component.
 */

/** A model round — one request/response with the LLM. */
export interface ChatRoundStep {
  kind: 'round';
  /** 0-based, stable for the turn; also the React key. */
  index: number;
  toolsOffered: number;
  /** Absent while the round is still open. */
  finishReason?: string;
  toolCalls?: number;
}

/** One tool invocation, from request to result. */
export interface ChatToolStep {
  kind: 'tool';
  id: string;
  name: string;
  /** Truncated by the server for the collapsed row. */
  argsPreview: string;
  /** The complete argument JSON, for the expanded detail. */
  args?: string;
  argsTruncated?: boolean;
  /** `undefined` while the call is in flight. */
  status?: number;
  /** The dispatch layer's own truncation flag. */
  truncated?: boolean;
  /** The complete response body, for the expanded detail. */
  response?: string;
  /** True size of the response BEFORE any cap — so a capped row can still say
   *  how much there really was. */
  bytes?: number;
  responseTruncated?: boolean;
}

export type ChatStep = ChatRoundStep | ChatToolStep;

/** How much of the machinery a turn renders.
 *
 *  Captured PER MESSAGE when its turn starts rather than read live, so toggling
 *  mid-session cannot rewrite history: a turn keeps the level it was produced
 *  under, and the change applies from the next message onwards. */
export type ChatViewLevel = 'clean' | 'verbose';

export const VIEW_LEVELS: ChatViewLevel[] = ['clean', 'verbose'];

export function isViewLevel(value: unknown): value is ChatViewLevel {
  return value === 'clean' || value === 'verbose';
}

/** Append a round marker. */
export function appendRoundStart(
  steps: ChatStep[],
  ev: { index: number; toolsOffered: number }
): ChatStep[] {
  return [...steps, { kind: 'round', index: ev.index, toolsOffered: ev.toolsOffered }];
}

/** Close the matching open round.
 *
 * Matched by `index` rather than by "the last round": frames are ordered, but
 * matching on identity means a duplicate or out-of-order close cannot silently
 * stamp its finish reason onto the wrong round. An unmatched close is dropped —
 * a stray frame must not invent a round that never started.
 */
export function applyRoundEnd(
  steps: ChatStep[],
  ev: { index: number; finishReason: string; toolCalls: number }
): ChatStep[] {
  let matched = false;
  const next = steps.map((step) => {
    // An ALREADY-CLOSED round is skipped: a round settles once, so a duplicate or
    // replayed frame cannot rewrite why it ended.
    if (matched || step.kind !== 'round' || step.index !== ev.index) return step;
    if (step.finishReason != null) return step;
    matched = true;
    return { ...step, finishReason: ev.finishReason, toolCalls: ev.toolCalls };
  });
  return matched ? next : steps;
}

/** Append a tool call in its arrival position. */
export function appendToolCall(
  steps: ChatStep[],
  ev: { id: string; name: string; argsPreview: string; args?: string; argsTruncated?: boolean }
): ChatStep[] {
  return [
    ...steps,
    {
      kind: 'tool',
      id: ev.id,
      name: ev.name,
      argsPreview: ev.argsPreview,
      args: ev.args,
      argsTruncated: ev.argsTruncated,
    },
  ];
}

/** Fold a result onto its call.
 *
 * Matched by tool-call id, so a result can never land on a different call even
 * when several run concurrently in one round. A result with no matching call is
 * appended as its own row rather than dropped: an orphan result still means a
 * tool ran, and silently losing it would under-report the work.
 */
export function applyToolResult(
  steps: ChatStep[],
  ev: {
    id: string;
    name: string;
    status: number;
    truncated: boolean;
    response?: string;
    bytes?: number;
    responseTruncated?: boolean;
  }
): ChatStep[] {
  // `seen` and `matched` are distinct: a call that is present but already settled
  // must neither be overwritten NOR fall through to the orphan append below, which
  // would show the same call twice.
  let seen = false;
  let matched = false;
  const next = steps.map((step) => {
    if (matched || step.kind !== 'tool' || step.id !== ev.id) return step;
    seen = true;
    // A call that already has its result is settled; a repeat frame must not
    // overwrite the outcome the user has already been shown.
    if (step.status != null) return step;
    matched = true;
    return {
      ...step,
      status: ev.status,
      truncated: ev.truncated,
      response: ev.response,
      bytes: ev.bytes,
      responseTruncated: ev.responseTruncated,
    };
  });
  if (matched) return next;
  if (seen) return steps;
  return [
    ...steps,
    {
      kind: 'tool',
      id: ev.id,
      name: ev.name,
      argsPreview: '',
      status: ev.status,
      truncated: ev.truncated,
      response: ev.response,
      bytes: ev.bytes,
      responseTruncated: ev.responseTruncated,
    },
  ];
}

/** The tool steps only — what a CLEAN turn summarises and the old chip row showed. */
export function toolSteps(steps: ChatStep[]): ChatToolStep[] {
  return steps.filter((step): step is ChatToolStep => step.kind === 'tool');
}

/** Pretty-print a JSON payload for the expanded detail, falling back to the raw
 *  string when it will not parse — a truncated payload is deliberately NOT valid
 *  JSON, and showing it raw beats showing nothing. */
export function formatPayload(raw: string | undefined): string {
  if (raw == null || raw === '') return '';
  try {
    return JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    return raw;
  }
}

/** Human-readable byte size for a row's summary. */
export function formatBytes(bytes: number | undefined): string {
  if (bytes == null) return '';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
