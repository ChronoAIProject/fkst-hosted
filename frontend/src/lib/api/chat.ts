// Streaming client for `POST /api/v1/chat`.
//
// `EventSource` cannot set an `Authorization` header, so the SSE stream is
// hand-rolled: an `apiFetch` POST, then `response.body.getReader()` decoded
// through a streaming `TextDecoder` and split into frames. Like every sibling in
// this directory it takes the caller's `apiFetch` as a dependency and never
// imports auth state itself.

import type { ApiFetch } from './canvas';
import { readErrorMessage } from './canvas';

/** A session the turn identified, mirroring the backend's `SessionRef` field for
 *  field (snake_case included) so nothing is lost in translation. */
export interface ChatSessionRef {
  owner: string;
  name: string;
  session_id?: string;
  trigger_number: number;
  title?: string;
  status_label?: string;
}

/** One frame of the response stream. `type` is the discriminant the backend sets. */
export type ChatStreamEvent =
  | { type: 'delta'; text: string }
  | { type: 'round_start'; index: number; tools_offered: number }
  | { type: 'round_end'; index: number; finish_reason: string; tool_calls: number }
  | {
      type: 'tool_call';
      id: string;
      name: string;
      args_preview: string;
      args?: string;
      args_truncated?: boolean;
    }
  | {
      type: 'tool_result';
      id: string;
      name: string;
      status: number;
      truncated: boolean;
      response?: string;
      bytes?: number;
      response_truncated?: boolean;
    }
  | { type: 'action_proposal'; proposal: unknown }
  | { type: 'data_card'; card: unknown }
  | { type: 'done'; finish_reason: string; session_refs: ChatSessionRef[] }
  | { type: 'error'; code: string; message: string };

/** Callbacks `streamChat` drives. Exactly one terminal callback — `onDone` or
 *  `onError` — always fires, so a caller never has to infer completion. */
export interface StreamChatHandlers {
  onDelta(text: string): void;
  /** A model round opened. Pairs with `onRoundEnd`, except when the turn dies
   *  inside the round — the server does not falsely close one. */
  onRoundStart(ev: { index: number; toolsOffered: number }): void;
  onRoundEnd(ev: { index: number; finishReason: string; toolCalls: number }): void;
  onToolCall(ev: {
    id: string;
    name: string;
    argsPreview: string;
    args?: string;
    argsTruncated?: boolean;
  }): void;
  onToolResult(ev: {
    id: string;
    name: string;
    status: number;
    truncated: boolean;
    response?: string;
    bytes?: number;
    responseTruncated?: boolean;
  }): void;
  onActionProposal(proposal: unknown): void;
  onDataCard(card: unknown): void;
  onDone(ev: { finishReason: string; sessionRefs: ChatSessionRef[] }): void;
  onError(err: { code: string; message: string; retryAfterSeconds?: number }): void;
}

/** Stable error codes the UI maps to copy. The transport-level ones are ours; the
 *  rest come straight from the backend's `error` frames. */
export const CHAT_ERROR_CODES = {
  /** The response could not be interpreted as SSE (bad frame, missing body). */
  protocol: 'protocol',
  /** A turn is already in flight, or global capacity is saturated. */
  rateLimited: 'rate_limited',
  /** Chat is not configured, or the replica is not ready. */
  unavailable: 'unavailable',
  /** The request never reached the server, or the network dropped. */
  network: 'network',
  /** The caller is not signed in (after `apiFetch`'s own refresh retry). */
  unauthorized: 'unauthorized',
  /** Anything else the server rejected before the stream started. */
  request: 'request',
} as const;

/** Map a pre-stream HTTP failure onto a stable code. */
function codeForStatus(status: number): string {
  if (status === 401) return CHAT_ERROR_CODES.unauthorized;
  if (status === 429) return CHAT_ERROR_CODES.rateLimited;
  if (status === 503) return CHAT_ERROR_CODES.unavailable;
  return CHAT_ERROR_CODES.request;
}

/** Split a buffer into complete SSE frames, returning the leftover.
 *
 *  Frames are blank-line delimited. CRLF is tolerated because a proxy may rewrite
 *  line endings, and a stray `\r` would otherwise corrupt every payload. */
function splitFrames(buffer: string): { frames: string[]; rest: string } {
  const normalized = buffer.replace(/\r\n/g, '\n');
  const parts = normalized.split('\n\n');
  // The last part is either incomplete or empty; either way it is carried over.
  const rest = parts.pop() ?? '';
  return { frames: parts, rest };
}

/** Extract one frame's `data:` payload, concatenating multiple `data:` lines.
 *
 *  Returns `null` for a frame with NO data line — an SSE comment / keep-alive
 *  (`: ...`), which the backend sends every 15s. Those are skipped silently: they
 *  are the protocol working, not a problem. */
function framePayload(frame: string): string | null {
  const lines = frame.split('\n');
  const data = lines
    .filter((line) => line.startsWith('data:'))
    .map((line) => line.slice('data:'.length).trimStart());
  return data.length === 0 ? null : data.join('');
}

/**
 * Run one chat turn, calling `handlers` as the stream arrives.
 *
 * `signal` aborts both the fetch and the reader, so stopping a turn (or closing
 * the panel) really does stop the work rather than leaving a read pending.
 */
export async function streamChat(
  apiFetch: ApiFetch,
  body: { messages: { role: 'user' | 'assistant'; content: string }[] },
  handlers: StreamChatHandlers,
  signal: AbortSignal,
  broaderToken?: string | null
): Promise<void> {
  let response: Response;
  try {
    response = await apiFetch('/api/v1/chat', {
      method: 'POST',
      signal,
      headers: {
        'Content-Type': 'application/json',
        // Ask for SSE explicitly, so a proxy that content-negotiates does not
        // buffer the response into a single blob.
        Accept: 'text/event-stream',
        ...(broaderToken ? { 'X-Github-Broader-Token': broaderToken } : {}),
      },
      body: JSON.stringify(body),
    });
  } catch (error) {
    // An abort is the user's own doing, not a failure to report.
    if (signal.aborted) return;
    handlers.onError({
      code: CHAT_ERROR_CODES.network,
      message: error instanceof Error ? error.message : 'request failed',
    });
    return;
  }

  if (!response.ok) {
    const message = await readErrorMessage(response);
    const retryAfter = Number(response.headers?.get?.('Retry-After') ?? '');
    handlers.onError({
      code: codeForStatus(response.status),
      message: message ?? `chat request failed: ${response.status}`,
      ...(Number.isFinite(retryAfter) && retryAfter > 0 ? { retryAfterSeconds: retryAfter } : {}),
    });
    return;
  }

  // jsdom and some proxies can hand back a bodyless response; that is a protocol
  // problem, not a silent no-op.
  if (response.body == null) {
    handlers.onError({
      code: CHAT_ERROR_CODES.protocol,
      message: 'the chat response carried no stream',
    });
    return;
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';
  const abortRead = () => reader.cancel().catch(() => {});
  signal.addEventListener('abort', abortRead, { once: true });

  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      // `stream: true` keeps a multi-byte character split across two chunks intact.
      buffer += decoder.decode(value, { stream: true });
      const { frames, rest } = splitFrames(buffer);
      buffer = rest;
      for (const frame of frames) {
        const payload = framePayload(frame);
        if (payload == null || payload === '') continue;

        let event: ChatStreamEvent;
        try {
          event = JSON.parse(payload) as ChatStreamEvent;
        } catch {
          // A `data:` payload we cannot parse means we are mis-reading the stream.
          // Fail loudly and stop: continuing would silently drop the answer.
          handlers.onError({
            code: CHAT_ERROR_CODES.protocol,
            message: 'the assistant sent an unreadable response',
          });
          await abortRead();
          return;
        }
        dispatch(event, handlers);
        if (event.type === 'done' || event.type === 'error') {
          await abortRead();
          return;
        }
      }
    }
  } catch (error) {
    if (signal.aborted) return;
    handlers.onError({
      code: CHAT_ERROR_CODES.network,
      message: error instanceof Error ? error.message : 'the stream ended unexpectedly',
    });
  }
}

/** Route one decoded event to its handler. An unknown `type` is ignored rather
 *  than treated as a protocol error: a newer server adding a frame kind must not
 *  break an older client mid-answer. */
function dispatch(event: ChatStreamEvent, handlers: StreamChatHandlers) {
  switch (event.type) {
    case 'delta':
      handlers.onDelta(event.text);
      return;
    case 'round_start':
      handlers.onRoundStart({ index: event.index, toolsOffered: event.tools_offered });
      return;
    case 'round_end':
      handlers.onRoundEnd({
        index: event.index,
        finishReason: event.finish_reason,
        toolCalls: event.tool_calls,
      });
      return;
    case 'tool_call':
      handlers.onToolCall({
        id: event.id,
        name: event.name,
        argsPreview: event.args_preview,
        args: event.args,
        argsTruncated: event.args_truncated,
      });
      return;
    case 'tool_result':
      handlers.onToolResult({
        id: event.id,
        name: event.name,
        status: event.status,
        truncated: event.truncated,
        response: event.response,
        bytes: event.bytes,
        responseTruncated: event.response_truncated,
      });
      return;
    case 'action_proposal':
      handlers.onActionProposal(event.proposal);
      return;
    case 'data_card':
      handlers.onDataCard(event.card);
      return;
    case 'done':
      handlers.onDone({
        finishReason: event.finish_reason,
        sessionRefs: Array.isArray(event.session_refs) ? event.session_refs : [],
      });
      return;
    case 'error':
      handlers.onError({ code: event.code, message: event.message });
      return;
    default:
      return;
  }
}
