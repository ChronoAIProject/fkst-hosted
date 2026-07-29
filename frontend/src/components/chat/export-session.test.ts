import { describe, it, expect, vi, afterEach } from 'vitest';
import {
  buildSessionExport,
  downloadSessionExport,
  EXPORT_SCHEMA,
  exportFilename,
  groupRounds,
} from './export-session';
import type { ChatMessage } from './chat-context';
import type { ChatStep } from './steps';

const AT = '2026-07-29T07:20:32.123Z';

const STEPS: ChatStep[] = [
  { kind: 'round', index: 0, toolsOffered: 17, finishReason: 'tool_calls', toolCalls: 1 },
  {
    kind: 'tool',
    id: 't1',
    name: 'get_overview',
    argsPreview: '{}',
    args: '{"account":"acme"}',
    status: 200,
    response: '{"repos":["a"]}',
    bytes: 15,
  },
  { kind: 'round', index: 1, toolsOffered: 17, finishReason: 'stop', toolCalls: 0 },
];

const MESSAGES: ChatMessage[] = [
  { id: 'u1', role: 'user', content: 'what do I have?' },
  {
    id: 'a1',
    role: 'assistant',
    content: 'You have one repo.',
    viewLevel: 'clean',
    steps: STEPS,
  },
];

describe('groupRounds', () => {
  it('attributes each call to the round that was open when it arrived', () => {
    const rounds = groupRounds(STEPS);
    expect(rounds).toHaveLength(2);
    expect(rounds[0]).toMatchObject({ index: 0, tools_offered: 17, finish_reason: 'tool_calls' });
    expect(rounds[0]!.tool_calls).toHaveLength(1);
    expect(rounds[1]!.tool_calls).toHaveLength(0);
  });

  it('keeps a call that arrived before any round rather than dropping it', () => {
    // Only reachable from a malformed stream, but an export must not quietly lose
    // evidence that a tool ran.
    const rounds = groupRounds([
      { kind: 'tool', id: 'x', name: 'get_overview', argsPreview: '{}', status: 200 },
    ]);
    expect(rounds).toHaveLength(1);
    expect(rounds[0]!.index).toBe(-1);
    expect(rounds[0]!.tool_calls).toHaveLength(1);
  });
});

describe('buildSessionExport', () => {
  it('carries the FULL detail even though the message rendered as CLEAN', () => {
    // The view level is a display concern; an export that omitted what the user
    // could not see would be useless for attaching to a bug report.
    const doc = buildSessionExport(MESSAGES, AT);
    expect(doc.schema).toBe(EXPORT_SCHEMA);
    expect(doc.exported_at).toBe(AT);

    const assistant = doc.messages[1]!;
    expect(assistant).toMatchObject({ view_level: 'clean' });
    const call = (assistant as { rounds: { tool_calls: unknown[] }[] }).rounds[0]!.tool_calls[0];
    expect(call).toMatchObject({
      name: 'get_overview',
      args: { account: 'acme' },
      response: { repos: ['a'] },
      status: 200,
      bytes: 15,
    });
  });

  it('keeps a truncated payload as its raw string rather than failing', () => {
    const doc = buildSessionExport(
      [
        {
          id: 'a1',
          role: 'assistant',
          content: '',
          steps: [
            {
              kind: 'tool',
              id: 't1',
              name: 'get_overview',
              argsPreview: '{}',
              status: 200,
              response: '{"partial"',
              responseTruncated: true,
              bytes: 14173,
            },
          ],
        },
      ],
      AT
    );
    const call = (doc.messages[0] as unknown as { rounds: { tool_calls: Record<string, unknown>[] }[] })
      .rounds[0]!.tool_calls[0]!;
    expect(call.response).toBe('{"partial"');
    expect(call.response_truncated).toBe(true);
    // The true size survives, so the reader knows how much was cut.
    expect(call.bytes).toBe(14173);
  });

  it('records an interrupted turn as interrupted', () => {
    const doc = buildSessionExport(
      [{ id: 'a1', role: 'assistant', content: 'partial', interrupted: true }],
      AT
    );
    expect(doc.messages[0]).toMatchObject({ interrupted: true, content: 'partial' });
  });

  it('produces a well-formed document for an empty session rather than throwing', () => {
    const doc = buildSessionExport([], AT);
    expect(doc.messages).toEqual([]);
    expect(() => JSON.stringify(doc)).not.toThrow();
  });

  it('omits the rounds key entirely for a turn that used no machinery', () => {
    const doc = buildSessionExport([{ id: 'a1', role: 'assistant', content: 'hi' }], AT);
    expect(doc.messages[0]).not.toHaveProperty('rounds');
  });
});

describe('exportFilename', () => {
  it('is filesystem-safe and timestamped so exports do not collide', () => {
    const name = exportFilename(AT);
    expect(name).toBe('fkst-orchestrator-2026-07-29T07-20-32-123Z.json');
    expect(name).not.toMatch(/[:]/);
    expect(exportFilename('2026-07-29T07:20:33.000Z')).not.toBe(name);
  });
});

describe('downloadSessionExport', () => {
  afterEach(() => vi.restoreAllMocks());

  it('revokes the object URL so the transcript is not pinned in memory', () => {
    const create = vi.fn(() => 'blob:fake');
    const revoke = vi.fn();
    vi.stubGlobal('URL', { ...URL, createObjectURL: create, revokeObjectURL: revoke });
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});

    downloadSessionExport(MESSAGES, AT);

    expect(click).toHaveBeenCalledOnce();
    expect(create).toHaveBeenCalledOnce();
    expect(revoke).toHaveBeenCalledWith('blob:fake');
    // The anchor must not be left in the document.
    expect(document.querySelector('a[download]')).toBeNull();
    vi.unstubAllGlobals();
  });

  it('still revokes the URL when the click throws', () => {
    const revoke = vi.fn();
    vi.stubGlobal('URL', {
      ...URL,
      createObjectURL: vi.fn(() => 'blob:fake'),
      revokeObjectURL: revoke,
    });
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {
      throw new Error('blocked');
    });

    expect(() => downloadSessionExport(MESSAGES, AT)).toThrow('blocked');
    expect(revoke).toHaveBeenCalledWith('blob:fake');
    vi.unstubAllGlobals();
  });
});
