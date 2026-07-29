import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { charsPerTick, prefersReducedMotion, TypewriterQueue } from './typewriter';

describe('charsPerTick', () => {
  it('drains a backlog inside the window rather than trickling', () => {
    // 700ms window at a 16ms tick is ~43 ticks; 430 chars therefore needs ~10 per tick.
    expect(charsPerTick(430, 16, 700, 24)).toBe(10);
  });

  it('never releases less than one character', () => {
    expect(charsPerTick(1, 16, 700, 24)).toBe(1);
  });

  it('caps a huge burst so it still animates', () => {
    expect(charsPerTick(100_000, 16, 700, 24)).toBe(24);
  });
});

describe('TypewriterQueue', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  /** Collect revealed slices into one string, as the transcript would. */
  function sink() {
    const parts: string[] = [];
    return { parts, write: (slice: string) => parts.push(slice) };
  }

  it('reveals text progressively rather than in one chunk', () => {
    const { parts, write } = sink();
    const queue = new TypewriterQueue(write, { intervalMs: 16, drainWindowMs: 700 });
    queue.push('hello world, this is a streamed answer');

    // Nothing is revealed synchronously: the whole point is that a big delta does not
    // appear the instant it arrives.
    expect(parts).toEqual([]);

    vi.advanceTimersByTime(16);
    expect(parts.length).toBe(1);
    expect(parts.join('').length).toBeLessThan('hello world, this is a streamed answer'.length);

    vi.advanceTimersByTime(2000);
    expect(parts.join('')).toBe('hello world, this is a streamed answer');
    // More than one slice — i.e. it really animated.
    expect(parts.length).toBeGreaterThan(1);
  });

  it('preserves order across several pushes', () => {
    const { parts, write } = sink();
    const queue = new TypewriterQueue(write, { intervalMs: 16, drainWindowMs: 100 });
    queue.push('one ');
    queue.push('two ');
    queue.push('three');
    vi.advanceTimersByTime(2000);
    expect(parts.join('')).toBe('one two three');
  });

  it('accepts a push made while it is already draining', () => {
    const { parts, write } = sink();
    const queue = new TypewriterQueue(write, { intervalMs: 16, drainWindowMs: 100 });
    queue.push('first ');
    vi.advanceTimersByTime(16);
    queue.push('second');
    vi.advanceTimersByTime(2000);
    expect(parts.join('')).toBe('first second');
  });

  it('runs the completion only once the queue has drained', () => {
    const { write } = sink();
    const queue = new TypewriterQueue(write, { intervalMs: 16, drainWindowMs: 700 });
    const done = vi.fn();
    queue.push('a fairly long answer that takes several ticks to reveal completely');
    queue.finish(done);

    // The wire is done, but the reader is not — dropping the caret here would contradict
    // the text still appearing.
    expect(done).not.toHaveBeenCalled();
    vi.advanceTimersByTime(3000);
    expect(done).toHaveBeenCalledTimes(1);
  });

  it('completes immediately when nothing is queued', () => {
    const { write } = sink();
    const queue = new TypewriterQueue(write, {});
    const done = vi.fn();
    queue.finish(done);
    expect(done).toHaveBeenCalledTimes(1);
  });

  it('flush reveals the remainder at once and completes', () => {
    const { parts, write } = sink();
    const queue = new TypewriterQueue(write, { intervalMs: 16, drainWindowMs: 5000 });
    const done = vi.fn();
    queue.push('the whole remaining answer');
    queue.finish(done);
    queue.flush();
    // The user pressed stop: they want what arrived, not an animation of it.
    expect(parts.join('')).toBe('the whole remaining answer');
    expect(done).toHaveBeenCalledTimes(1);
    expect(queue.pending).toBe(false);
  });

  it('cancel drops the queue and never completes', () => {
    const { parts, write } = sink();
    const queue = new TypewriterQueue(write, { intervalMs: 16 });
    const done = vi.fn();
    queue.push('abandoned text');
    queue.finish(done);
    queue.cancel();
    vi.advanceTimersByTime(5000);
    expect(parts).toEqual([]);
    expect(done).not.toHaveBeenCalled();
  });

  it('reveals instantly when reduced motion is requested', () => {
    const { parts, write } = sink();
    const queue = new TypewriterQueue(write, { instant: true });
    queue.push('no animation please');
    // Synchronous: an animated reveal is precisely what this viewer asked not to see.
    expect(parts).toEqual(['no animation please']);
    expect(queue.pending).toBe(false);
  });

  it('never splits a surrogate pair', () => {
    const { parts, write } = sink();
    // One char per tick, so every emoji lands on a slice boundary if the split is wrong.
    const queue = new TypewriterQueue(write, { intervalMs: 16, drainWindowMs: 100_000 });
    queue.push('🚀🛰️✅');
    vi.advanceTimersByTime(5000);
    expect(parts.join('')).toBe('🚀🛰️✅');
    expect(parts.join('')).not.toContain('�');
  });

  it('ignores an empty push', () => {
    const { parts, write } = sink();
    const queue = new TypewriterQueue(write, {});
    queue.push('');
    expect(parts).toEqual([]);
    expect(queue.pending).toBe(false);
  });
});

describe('prefersReducedMotion', () => {
  it('is false when the query does not match', () => {
    vi.stubGlobal('matchMedia', vi.fn().mockReturnValue({ matches: false }));
    expect(prefersReducedMotion()).toBe(false);
    vi.unstubAllGlobals();
  });

  it('is true when the viewer asked for reduced motion', () => {
    vi.stubGlobal('matchMedia', vi.fn().mockReturnValue({ matches: true }));
    expect(prefersReducedMotion()).toBe(true);
    vi.unstubAllGlobals();
  });
});
