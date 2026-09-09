import { describe, it, expect } from 'vitest';
import {
  appendRoundStart,
  appendRoundText,
  appendToolCall,
  applyRoundEnd,
  applyToolResult,
  formatBytes,
  formatPayload,
  isViewLevel,
  toolSteps,
} from './steps';
import type { ChatStep } from './steps';

const round = (index: number, toolsOffered = 3): ChatStep => ({
  kind: 'round',
  index,
  toolsOffered,
});
const tool = (id: string, name = 'get_overview'): ChatStep => ({
  kind: 'tool',
  id,
  name,
  argsPreview: '{}',
});

describe('step reducers', () => {
  it('keeps steps in arrival order, because order IS the record', () => {
    let steps: ChatStep[] = [];
    steps = appendRoundStart(steps, { index: 0, toolsOffered: 3 });
    steps = appendToolCall(steps, { id: 'a', name: 'get_overview', argsPreview: '{}' });
    steps = appendToolCall(steps, { id: 'b', name: 'list_repos', argsPreview: '{}' });
    steps = appendRoundStart(steps, { index: 1, toolsOffered: 3 });
    expect(steps.map((s) => (s.kind === 'round' ? `r${s.index}` : `t${s.id}`))).toEqual([
      'r0',
      'ta',
      'tb',
      'r1',
    ]);
  });

  it('closes the round with the MATCHING index, not merely the last one', () => {
    const steps = applyRoundEnd([round(0), round(1)], {
      index: 0,
      finishReason: 'tool_calls',
      toolCalls: 2,
    });
    expect(steps[0]).toMatchObject({ index: 0, finishReason: 'tool_calls', toolCalls: 2 });
    expect(steps[1]).not.toHaveProperty('finishReason');
  });

  it('drops a close for a round that never started rather than inventing one', () => {
    const steps: ChatStep[] = [round(0)];
    const next = applyRoundEnd(steps, { index: 7, finishReason: 'stop', toolCalls: 0 });
    expect(next).toEqual(steps);
  });

  it('closes a round only once, so a duplicate frame cannot restamp it', () => {
    let steps = applyRoundEnd([round(0)], { index: 0, finishReason: 'tool_calls', toolCalls: 2 });
    steps = applyRoundEnd(steps, { index: 0, finishReason: 'stop', toolCalls: 9 });
    expect(steps[0]).toMatchObject({ finishReason: 'tool_calls', toolCalls: 2 });
  });

  it('folds a result onto its own call even when several are in flight', () => {
    const steps = applyToolResult([tool('a'), tool('b', 'list_repos')], {
      id: 'b',
      name: 'list_repos',
      status: 200,
      truncated: false,
      response: '{"ok":true}',
      bytes: 11,
    });
    expect(steps).toHaveLength(2);
    expect(steps[0]).not.toHaveProperty('status');
    expect(steps[1]).toMatchObject({ id: 'b', status: 200, response: '{"ok":true}', bytes: 11 });
  });

  it('appends an orphan result rather than losing evidence that a tool ran', () => {
    const steps = applyToolResult([], {
      id: 'ghost',
      name: 'get_overview',
      status: 200,
      truncated: false,
    });
    expect(steps).toHaveLength(1);
    expect(steps[0]).toMatchObject({ kind: 'tool', id: 'ghost', status: 200 });
  });

  it('folds a result onto one call only, so a repeat cannot double-count', () => {
    let steps = applyToolResult([tool('a')], {
      id: 'a',
      name: 'get_overview',
      status: 200,
      truncated: false,
    });
    steps = applyToolResult(steps, {
      id: 'a',
      name: 'get_overview',
      status: 500,
      truncated: false,
    });
    expect(steps).toHaveLength(1);
    expect(steps[0]).toMatchObject({ status: 200 });
  });

  it('folds a delta onto the round the SERVER named, not the round that is open', () => {
    // Round 0 is still the last one appended, but the delta says round 1.
    let steps: ChatStep[] = [round(0), round(1)];
    steps = appendRoundText(steps, { round: 1, text: 'answer' });
    expect(steps[0]).not.toHaveProperty('text');
    expect(steps[1]).toMatchObject({ index: 1, text: 'answer' });
  });

  it('accumulates successive deltas for the same round', () => {
    let steps: ChatStep[] = [round(0)];
    steps = appendRoundText(steps, { round: 0, text: 'Look' });
    steps = appendRoundText(steps, { round: 0, text: 'ing…' });
    expect(steps[0]).toMatchObject({ text: 'Looking…' });
  });

  it('drops text for a round that never started rather than misattributing it', () => {
    // Silently attaching it to the wrong round would be worse than losing it.
    const steps: ChatStep[] = [round(0)];
    expect(appendRoundText(steps, { round: 7, text: 'ghost' })).toEqual(steps);
  });

  it('ignores a delta with no round, so an older server cannot corrupt a round', () => {
    const steps: ChatStep[] = [round(0)];
    expect(appendRoundText(steps, { text: 'unattributed' })).toEqual(steps);
  });

  it('selects only the tool steps for the CLEAN summary', () => {
    expect(toolSteps([round(0), tool('a'), round(1), tool('b')])).toHaveLength(2);
  });
});

describe('payload formatting', () => {
  it('pretty-prints valid JSON', () => {
    expect(formatPayload('{"a":1}')).toBe('{\n  "a": 1\n}');
  });

  it('shows a truncated payload raw rather than showing nothing', () => {
    // A capped payload is deliberately NOT valid JSON; the row still has to render it.
    expect(formatPayload('{"a":1,"b"')).toBe('{"a":1,"b"');
  });

  it('renders an absent payload as empty rather than "undefined"', () => {
    expect(formatPayload(undefined)).toBe('');
    expect(formatPayload('')).toBe('');
  });

  it('scales byte sizes', () => {
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(14173)).toBe('13.8 KB');
    expect(formatBytes(2 * 1024 * 1024)).toBe('2.0 MB');
    expect(formatBytes(undefined)).toBe('');
  });
});

describe('view level', () => {
  it('accepts only the two real levels', () => {
    expect(isViewLevel('clean')).toBe(true);
    expect(isViewLevel('verbose')).toBe(true);
    expect(isViewLevel('loud')).toBe(false);
    expect(isViewLevel(null)).toBe(false);
  });
});
