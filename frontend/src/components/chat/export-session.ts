import type { ChatMessage } from './chat-context';
import type { ChatStep, ChatToolStep } from './steps';

/**
 * Exporting a session as JSON.
 *
 * The transcript already lives in the client, so this is a pure serialise +
 * client-side download: no server round-trip, and nothing is stored server-side.
 *
 * The document carries the FULL captured record — every round, every tool call's
 * parameters and response — regardless of the current CLEAN/VERBOSE view. The
 * view level is a display concern and is recorded PER MESSAGE rather than used to
 * filter, because an export that silently omitted what the user could not see
 * would be useless for the thing exports are for: attaching to a bug report.
 */

/** Versioned so the shape can evolve without a consumer guessing. */
export const EXPORT_SCHEMA = 'fkst-orchestrator-session/v1';

interface ExportedToolCall {
  name: string;
  /** Parsed where possible so the document is readable JSON rather than a string
   *  holding JSON; falls back to the raw string when it will not parse (a capped
   *  payload is deliberately not valid JSON). */
  args: unknown;
  args_truncated?: boolean;
  status?: number;
  bytes?: number;
  response: unknown;
  response_truncated?: boolean;
}

interface ExportedRound {
  index: number;
  tools_offered: number;
  finish_reason?: string;
  tool_calls: ExportedToolCall[];
}

/** Re-parse a payload for the document, keeping the raw string when it will not
 *  parse. `undefined` stays absent rather than becoming `null`. */
function payload(raw: string | undefined): unknown {
  if (raw == null || raw === '') return undefined;
  try {
    return JSON.parse(raw);
  } catch {
    return raw;
  }
}

function toExportedCall(step: ChatToolStep): ExportedToolCall {
  return {
    name: step.name,
    args: payload(step.args ?? step.argsPreview),
    ...(step.argsTruncated ? { args_truncated: true } : {}),
    ...(step.status != null ? { status: step.status } : {}),
    ...(step.bytes != null ? { bytes: step.bytes } : {}),
    response: payload(step.response),
    ...(step.responseTruncated ? { response_truncated: true } : {}),
  };
}

/**
 * Group a message's flat step list into rounds.
 *
 * Calls are attributed to the round that was open when they arrived, which is
 * what makes the document readable as the loop it was. Calls that arrive before
 * any round (only possible from a malformed stream) go into a synthetic round
 * with index `-1` rather than being dropped — an export must not quietly lose
 * evidence that a tool ran.
 */
export function groupRounds(steps: ChatStep[]): ExportedRound[] {
  const rounds: ExportedRound[] = [];
  let current: ExportedRound | null = null;
  for (const step of steps) {
    if (step.kind === 'round') {
      current = {
        index: step.index,
        tools_offered: step.toolsOffered,
        ...(step.finishReason != null ? { finish_reason: step.finishReason } : {}),
        tool_calls: [],
      };
      rounds.push(current);
      continue;
    }
    if (current == null) {
      current = { index: -1, tools_offered: 0, tool_calls: [] };
      rounds.push(current);
    }
    current.tool_calls.push(toExportedCall(step));
  }
  return rounds;
}

/** Build the export document. `exportedAt` is injected so the caller owns the
 *  clock — which also makes this deterministic to test. */
export function buildSessionExport(messages: ChatMessage[], exportedAt: string) {
  return {
    schema: EXPORT_SCHEMA,
    exported_at: exportedAt,
    messages: messages.map((message) => ({
      role: message.role,
      content: message.content,
      ...(message.viewLevel != null ? { view_level: message.viewLevel } : {}),
      ...(message.interrupted ? { interrupted: true } : {}),
      ...(message.tone != null ? { tone: message.tone } : {}),
      ...(message.steps != null && message.steps.length > 0
        ? { rounds: groupRounds(message.steps) }
        : {}),
    })),
  };
}

/** Timestamped so successive exports do not collide in the download folder. */
export function exportFilename(exportedAt: string): string {
  return `fkst-orchestrator-${exportedAt.replace(/[:.]/g, '-')}.json`;
}

/**
 * Trigger the download.
 *
 * The object URL is revoked immediately after the synthetic click: the blob is
 * already handed to the download, and leaving it alive pins the whole transcript
 * in memory for the life of the document.
 */
export function downloadSessionExport(messages: ChatMessage[], exportedAt: string): void {
  const blob = new Blob([JSON.stringify(buildSessionExport(messages, exportedAt), null, 2)], {
    type: 'application/json',
  });
  const url = URL.createObjectURL(blob);
  try {
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = exportFilename(exportedAt);
    document.body.appendChild(anchor);
    anchor.click();
    anchor.remove();
  } finally {
    URL.revokeObjectURL(url);
  }
}
