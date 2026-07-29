import { describe, it, expect, vi } from 'vitest';
import { streamChat } from './chat';
import type { StreamChatHandlers } from './chat';
import type { ApiFetch } from './canvas';

/** Collect every handler call, in order, as a readable trace. */
function recorder() {
  const trace: string[] = [];
  const handlers: StreamChatHandlers = {
    onDelta: (text) => trace.push(`delta:${text}`),
    onToolCall: ({ name }) => trace.push(`tool_call:${name}`),
    onToolResult: ({ name, status, truncated }) =>
      trace.push(`tool_result:${name}:${status}${truncated ? ':truncated' : ''}`),
    onActionProposal: (proposal) => trace.push(`proposal:${JSON.stringify(proposal)}`),
    onDataCard: (card) => trace.push(`card:${JSON.stringify(card)}`),
    onDone: ({ finishReason, sessionRefs }) =>
      trace.push(`done:${finishReason}:${sessionRefs.length}`),
    onError: ({ code, retryAfterSeconds }) =>
      trace.push(`error:${code}${retryAfterSeconds != null ? `:${retryAfterSeconds}` : ''}`),
  };
  return { trace, handlers };
}

/** A `Response` whose body streams the given chunks, in order. */
function streamingResponse(
  chunks: string[],
  init: { status?: number; headers?: Record<string, string> } = {}
) {
  const encoder = new TextEncoder();
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      chunks.forEach((chunk) => controller.enqueue(encoder.encode(chunk)));
      controller.close();
    },
  });
  const status = init.status ?? 200;
  return {
    ok: status >= 200 && status < 300,
    status,
    body,
    headers: { get: (name: string) => init.headers?.[name] ?? null },
    json: async () => ({}),
  } as unknown as Response;
}

/** A non-streaming error `Response` carrying an error envelope. */
function errorResponse(status: number, message: string, headers: Record<string, string> = {}) {
  return {
    ok: false,
    status,
    body: null,
    headers: { get: (name: string) => headers[name] ?? null },
    json: async () => ({ error: 'e', message }),
  } as unknown as Response;
}

const body = { messages: [{ role: 'user' as const, content: 'hi' }] };
const frame = (payload: unknown) => `data: ${JSON.stringify(payload)}\n\n`;

describe('streamChat — request shape', () => {
  it('POSTs the documented path with the SSE headers and the messages body', async () => {
    const apiFetch = vi.fn(async () => streamingResponse([])) as unknown as ApiFetch;
    const { handlers } = recorder();
    await streamChat(apiFetch, body, handlers, new AbortController().signal);

    expect(apiFetch).toHaveBeenCalledTimes(1);
    const [path, init] = (apiFetch as unknown as ReturnType<typeof vi.fn>).mock.calls[0]!;
    expect(path).toBe('/api/v1/chat');
    expect(init.method).toBe('POST');
    expect(init.headers['Content-Type']).toBe('application/json');
    // Asked for explicitly, so a content-negotiating proxy does not buffer the
    // response into one blob.
    expect(init.headers['Accept']).toBe('text/event-stream');
    expect(JSON.parse(init.body)).toEqual(body);
    expect(init.headers['X-Github-Broader-Token']).toBeUndefined();
  });

  it('forwards the broader-visibility token when there is one', async () => {
    const apiFetch = vi.fn(async () => streamingResponse([])) as unknown as ApiFetch;
    const { handlers } = recorder();
    await streamChat(apiFetch, body, handlers, new AbortController().signal, 'gho_broader');
    const [, init] = (apiFetch as unknown as ReturnType<typeof vi.fn>).mock.calls[0]!;
    expect(init.headers['X-Github-Broader-Token']).toBe('gho_broader');
  });
});

describe('streamChat — frame parsing', () => {
  it('dispatches every event kind in order', async () => {
    const apiFetch = vi.fn(async () =>
      streamingResponse([
        frame({ type: 'delta', text: 'Hello' }),
        frame({ type: 'tool_call', id: 't1', name: 'get_overview', args_preview: '{}' }),
        frame({
          type: 'tool_result',
          id: 't1',
          name: 'get_overview',
          status: 200,
          truncated: false,
        }),
        frame({ type: 'data_card', card: { kind: 'environments', profiles: [] } }),
        frame({ type: 'action_proposal', proposal: { kind: 'stop_session' } }),
        frame({
          type: 'done',
          finish_reason: 'stop',
          session_refs: [{ owner: 'a', name: 'b', trigger_number: 7 }],
        }),
      ])
    ) as unknown as ApiFetch;
    const { trace, handlers } = recorder();
    await streamChat(apiFetch, body, handlers, new AbortController().signal);

    expect(trace).toEqual([
      'delta:Hello',
      'tool_call:get_overview',
      'tool_result:get_overview:200',
      'card:{"kind":"environments","profiles":[]}',
      'proposal:{"kind":"stop_session"}',
      'done:stop:1',
    ]);
  });

  it('reassembles a frame split across chunk boundaries', async () => {
    const payload = frame({ type: 'delta', text: 'split' });
    const half = Math.floor(payload.length / 2);
    const apiFetch = vi.fn(async () =>
      streamingResponse([payload.slice(0, half), payload.slice(half)])
    ) as unknown as ApiFetch;
    const { trace, handlers } = recorder();
    await streamChat(apiFetch, body, handlers, new AbortController().signal);
    expect(trace).toEqual(['delta:split']);
  });

  it('keeps a multi-byte character split across chunks intact', async () => {
    // The streaming TextDecoder is what makes this work; a per-chunk decode would
    // produce a replacement character.
    const payload = frame({ type: 'delta', text: '日本語' });
    const bytes = new TextEncoder().encode(payload);
    const cut = payload.indexOf('日') + 1;
    const decoder = new TextDecoder();
    const encoder = new TextEncoder();
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(bytes.slice(0, cut));
        controller.enqueue(bytes.slice(cut));
        controller.close();
      },
    });
    void decoder;
    void encoder;
    const apiFetch = vi.fn(async () => ({
      ok: true,
      status: 200,
      body: stream,
      headers: { get: () => null },
      json: async () => ({}),
    })) as unknown as ApiFetch;
    const { trace, handlers } = recorder();
    await streamChat(apiFetch, body, handlers, new AbortController().signal);
    expect(trace).toEqual(['delta:日本語']);
  });

  it('tolerates CRLF line endings', async () => {
    const apiFetch = vi.fn(async () =>
      streamingResponse([`data: ${JSON.stringify({ type: 'delta', text: 'crlf' })}\r\n\r\n`])
    ) as unknown as ApiFetch;
    const { trace, handlers } = recorder();
    await streamChat(apiFetch, body, handlers, new AbortController().signal);
    expect(trace).toEqual(['delta:crlf']);
  });

  it('concatenates multiple data lines in one frame', async () => {
    const json = JSON.stringify({ type: 'delta', text: 'joined' });
    const split = Math.floor(json.length / 2);
    const apiFetch = vi.fn(async () =>
      streamingResponse([`data: ${json.slice(0, split)}\ndata: ${json.slice(split)}\n\n`])
    ) as unknown as ApiFetch;
    const { trace, handlers } = recorder();
    await streamChat(apiFetch, body, handlers, new AbortController().signal);
    expect(trace).toEqual(['delta:joined']);
  });

  it('skips keep-alive comment frames silently', async () => {
    // The backend sends one every 15s; treating it as a problem would break every
    // slow answer.
    const apiFetch = vi.fn(async () =>
      streamingResponse([
        ': keep-alive\n\n',
        frame({ type: 'delta', text: 'after' }),
        ': keep-alive\n\n',
        frame({ type: 'done', finish_reason: 'stop', session_refs: [] }),
      ])
    ) as unknown as ApiFetch;
    const { trace, handlers } = recorder();
    await streamChat(apiFetch, body, handlers, new AbortController().signal);
    expect(trace).toEqual(['delta:after', 'done:stop:0']);
  });

  it('stops reading after the terminal frame', async () => {
    const apiFetch = vi.fn(async () =>
      streamingResponse([
        frame({ type: 'done', finish_reason: 'stop', session_refs: [] }),
        frame({ type: 'delta', text: 'never' }),
      ])
    ) as unknown as ApiFetch;
    const { trace, handlers } = recorder();
    await streamChat(apiFetch, body, handlers, new AbortController().signal);
    expect(trace).toEqual(['done:stop:0']);
  });

  it('ignores an unknown frame kind rather than failing the turn', async () => {
    // A newer server adding a frame kind must not break an older client mid-answer.
    const apiFetch = vi.fn(async () =>
      streamingResponse([
        frame({ type: 'something_new', detail: 1 }),
        frame({ type: 'delta', text: 'still works' }),
        frame({ type: 'done', finish_reason: 'stop', session_refs: [] }),
      ])
    ) as unknown as ApiFetch;
    const { trace, handlers } = recorder();
    await streamChat(apiFetch, body, handlers, new AbortController().signal);
    expect(trace).toEqual(['delta:still works', 'done:stop:0']);
  });

  it('treats a malformed data payload as a protocol error and stops', async () => {
    const apiFetch = vi.fn(async () =>
      streamingResponse(['data: {not json\n\n', frame({ type: 'delta', text: 'never' })])
    ) as unknown as ApiFetch;
    const { trace, handlers } = recorder();
    await streamChat(apiFetch, body, handlers, new AbortController().signal);
    expect(trace).toEqual(['error:protocol']);
  });

  it('reports a missing body as a protocol error', async () => {
    const apiFetch = vi.fn(async () => ({
      ok: true,
      status: 200,
      body: null,
      headers: { get: () => null },
      json: async () => ({}),
    })) as unknown as ApiFetch;
    const { trace, handlers } = recorder();
    await streamChat(apiFetch, body, handlers, new AbortController().signal);
    expect(trace).toEqual(['error:protocol']);
  });

  it('defaults absent session refs to an empty list', async () => {
    const apiFetch = vi.fn(async () =>
      streamingResponse([frame({ type: 'done', finish_reason: 'stop' })])
    ) as unknown as ApiFetch;
    const { trace, handlers } = recorder();
    await streamChat(apiFetch, body, handlers, new AbortController().signal);
    expect(trace).toEqual(['done:stop:0']);
  });

  it('reports a truncated tool result', async () => {
    const apiFetch = vi.fn(async () =>
      streamingResponse([
        frame({
          type: 'tool_result',
          id: 't',
          name: 'tail_log_file',
          status: 200,
          truncated: true,
        }),
      ])
    ) as unknown as ApiFetch;
    const { trace, handlers } = recorder();
    await streamChat(apiFetch, body, handlers, new AbortController().signal);
    expect(trace).toEqual(['tool_result:tail_log_file:200:truncated']);
  });
});

describe('streamChat — pre-stream failures', () => {
  it('maps 429 to a rate-limit code carrying Retry-After', async () => {
    const apiFetch = vi.fn(async () =>
      errorResponse(429, 'a turn is already in flight', { 'Retry-After': '5' })
    ) as unknown as ApiFetch;
    const { trace, handlers } = recorder();
    await streamChat(apiFetch, body, handlers, new AbortController().signal);
    expect(trace).toEqual(['error:rate_limited:5']);
  });

  it('maps 503 to unavailable', async () => {
    const apiFetch = vi.fn(async () =>
      errorResponse(503, 'chat is not configured')
    ) as unknown as ApiFetch;
    const { trace, handlers } = recorder();
    await streamChat(apiFetch, body, handlers, new AbortController().signal);
    expect(trace).toEqual(['error:unavailable']);
  });

  it('maps 401 to unauthorized (after apiFetch has already retried)', async () => {
    const apiFetch = vi.fn(async () => errorResponse(401, 'no token')) as unknown as ApiFetch;
    const { trace, handlers } = recorder();
    await streamChat(apiFetch, body, handlers, new AbortController().signal);
    expect(trace).toEqual(['error:unauthorized']);
  });

  it('maps any other rejection to a request error', async () => {
    const apiFetch = vi.fn(async () => errorResponse(422, 'bad history')) as unknown as ApiFetch;
    const { trace, handlers } = recorder();
    await streamChat(apiFetch, body, handlers, new AbortController().signal);
    expect(trace).toEqual(['error:request']);
  });

  it('maps a thrown fetch to a network error', async () => {
    const apiFetch = vi.fn(async () => {
      throw new Error('offline');
    }) as unknown as ApiFetch;
    const { trace, handlers } = recorder();
    await streamChat(apiFetch, body, handlers, new AbortController().signal);
    expect(trace).toEqual(['error:network']);
  });
});

describe('streamChat — abort', () => {
  it('reports nothing when the caller aborted before the request settled', async () => {
    // An abort is the user's own doing, not a failure to tell them about.
    const controller = new AbortController();
    const apiFetch = vi.fn(async () => {
      controller.abort();
      throw new Error('aborted');
    }) as unknown as ApiFetch;
    const { trace, handlers } = recorder();
    await streamChat(apiFetch, body, handlers, controller.signal);
    expect(trace).toEqual([]);
  });

  it('cancels the reader when aborted mid-stream', async () => {
    const controller = new AbortController();
    let cancelled = false;
    const encoder = new TextEncoder();
    const stream = new ReadableStream<Uint8Array>({
      start(streamController) {
        streamController.enqueue(encoder.encode(frame({ type: 'delta', text: 'first' })));
        // Left open on purpose: the abort is what must end the read.
      },
      cancel() {
        cancelled = true;
      },
    });
    const apiFetch = vi.fn(async () => ({
      ok: true,
      status: 200,
      body: stream,
      headers: { get: () => null },
      json: async () => ({}),
    })) as unknown as ApiFetch;

    const { trace, handlers } = recorder();
    const pending = streamChat(apiFetch, body, handlers, controller.signal);
    // Let the first chunk land, then abort.
    await Promise.resolve();
    controller.abort();
    await pending;

    expect(trace).toEqual(['delta:first']);
    expect(cancelled).toBe(true);
  });
});
