/**
 * The chat transport seam: everything the UI needs from "whatever talks to the
 * backend", and nothing more.
 *
 * The panel, the transcript, and the composer depend only on this interface, so
 * the real SSE client drops in behind it with no component change — and the whole
 * surface stays reviewable (and testable) against `mockEchoTransport` before any
 * network exists.
 *
 * The handler shape mirrors the backend's SSE event protocol one-to-one on
 * purpose: an adapter that has to reshape events is an adapter that can lose one.
 */

import { streamChat } from '@/lib/api/chat';
import type { ApiFetch } from '@/lib/api/canvas';

/** One message on the wire. Deliberately narrower than the UI's message type:
 *  only user/assistant CONTENT is ever sent — local notices, pending
 *  placeholders, and tool events are display state, not conversation. */
export interface ChatTurnMessage {
  role: 'user' | 'assistant';
  content: string;
}

/** A session the turn's tool results identified, for a deep-linking card.
 *  Mirrors the backend's `SessionRef`, snake_case included, so no field is lost
 *  in translation. */
export interface SessionRef {
  owner: string;
  name: string;
  session_id?: string;
  trigger_number: number;
  title?: string;
  status_label?: string;
}

/** Callbacks a transport drives as a turn streams. */
export interface ChatTransportHandlers {
  onDelta(text: string, round?: number): void;
  /** A model round opened / closed — the orchestration loop made visible. */
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
  /** A structured rendering of the tool result that just landed. */
  onDataCard(card: unknown): void;
  onDone(ev: { finishReason: string; sessionRefs: SessionRef[] }): void;
  /** `retryAfterSeconds` rides along when the server advertised `Retry-After`,
   *  because "try again in 5s" is actionable where "try again" is not. */
  onError(err: { code: string; message: string; retryAfterSeconds?: number }): void;
}

/** Runs one conversation turn. Implementations must honor `signal` (the user can
 *  stop a turn, and closing the panel aborts) and must always end with exactly
 *  one terminal callback — `onDone` or `onError` — so the UI never has to infer
 *  completion. */
export interface ChatTransport {
  send(history: ChatTurnMessage[], handlers: ChatTransportHandlers, signal: AbortSignal): void;
}

/** The real transport: `POST /api/v1/chat` as a streamed SSE response.
 *
 *  A thin adapter by design — the handler names and shapes already line up with the
 *  wire protocol, so this maps them one-to-one and adds no interpretation of its
 *  own. `getBroaderToken` is read PER TURN, not captured, so connecting or
 *  disconnecting broader visibility takes effect on the next question rather than
 *  requiring a remount. */
export function sseChatTransport(
  apiFetch: ApiFetch,
  getBroaderToken?: () => string | null
): ChatTransport {
  return {
    send(history, handlers, signal) {
      void streamChat(
        apiFetch,
        { messages: history.map(({ role, content }) => ({ role, content })) },
        {
          onDelta: handlers.onDelta,
          onRoundStart: handlers.onRoundStart,
          onRoundEnd: handlers.onRoundEnd,
          onToolCall: handlers.onToolCall,
          onToolResult: handlers.onToolResult,
          onActionProposal: handlers.onActionProposal,
          onDataCard: handlers.onDataCard,
          onDone: handlers.onDone,
          onError: handlers.onError,
        },
        signal,
        getBroaderToken?.()
      );
    },
  };
}

/** Delay between mock chunks — slow enough to see streaming, fast enough that a
 *  test does not crawl. */
const MOCK_CHUNK_MS = 45;

/** The canned answer, split so it arrives in visible pieces. It deliberately
 *  includes a fenced code block and a list so the markdown path is exercised. */
const MOCK_CHUNKS = [
  'Here is what I can see so far.\n\n',
  'Your sessions are driven by **trigger issues**. ',
  'A session picks up any issue carrying one of its work labels, ',
  'and each one comes back as its own pull request.\n\n',
  'To start one, open an issue labeled `fkst-substrate-trigger` with a body like:\n\n',
  '```\n### Session Name\nsitebuilder\n\n### Work Label\nsite-build\n```\n\n',
  'Ask me about a specific repository and I will look it up.',
];

/**
 * A local transport that fakes a complete turn: two tool events, a streamed
 * markdown answer, and a clean `done`.
 *
 * It exists so the entire chat surface — every state the UI can be in — is
 * reviewable and testable with no backend, and it stays exported afterwards
 * because it remains the cheapest way to drive the UI in tests.
 */
export const mockEchoTransport: ChatTransport = {
  send(history, handlers, signal) {
    const timers: number[] = [];
    let cancelled = false;

    const stop = () => {
      cancelled = true;
      timers.forEach((id) => window.clearTimeout(id));
    };
    signal.addEventListener('abort', stop, { once: true });

    /** Queue one step; every step re-checks cancellation, because an abort
     *  between two timers must not deliver the next chunk. */
    const at = (delay: number, run: () => void) => {
      timers.push(
        window.setTimeout(() => {
          if (cancelled) return;
          run();
        }, delay)
      );
    };

    const toolId = 'mock-tool-1';
    // The mock walks the same shape as the real loop — round, call, result, round
    // close — so the timeline can be developed without a backend.
    const mockArgs = '{"query":"fkst"}';
    const mockResponse = '{"status":200,"body":{"matches":3}}';
    at(0, () => handlers.onRoundStart({ index: 0, toolsOffered: 1 }));
    at(MOCK_CHUNK_MS, () =>
      handlers.onToolCall({
        id: toolId,
        name: 'search_manual',
        argsPreview: mockArgs,
        args: mockArgs,
      })
    );
    at(MOCK_CHUNK_MS * 2, () =>
      handlers.onToolResult({
        id: toolId,
        name: 'search_manual',
        status: 200,
        truncated: false,
        response: mockResponse,
        bytes: mockResponse.length,
      })
    );
    at(MOCK_CHUNK_MS * 2, () =>
      handlers.onRoundEnd({ index: 0, finishReason: 'tool_calls', toolCalls: 1 })
    );

    MOCK_CHUNKS.forEach((chunk, index) => {
      at(MOCK_CHUNK_MS * (index + 3), () => handlers.onDelta(chunk));
    });

    at(MOCK_CHUNK_MS * (MOCK_CHUNKS.length + 3), () => {
      // Echo the question count so a reviewer can see the history really arrived.
      handlers.onDelta(`\n\n_(mock transport · ${history.length} message(s) sent)_`);
      handlers.onDone({ finishReason: 'stop', sessionRefs: [] });
    });
  },
};
