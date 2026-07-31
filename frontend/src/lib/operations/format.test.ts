import { describe, expect, it } from 'vitest';
import { activityCacheKey, sandboxCacheKey } from './keys';
import {
  argumentEntries,
  deliveryTone,
  displayLocation,
  elapsedSeconds,
  formatDurationMs,
  formatDurationSeconds,
  lifetimeDisplay,
  outcomeTone,
  remainingSeconds,
  sandboxTone,
  summarizeArguments,
  summarizeValue,
} from './format';
import { DEFAULT_ACTIVITY_FILTERS, DEFAULT_SANDBOX_FILTERS } from './state';

describe('duration formatting', () => {
  it('renders each magnitude in its own shape', () => {
    expect(formatDurationSeconds(42)).toBe('42s');
    expect(formatDurationSeconds(125)).toBe('2m 05s');
    expect(formatDurationSeconds(3725)).toBe('1h 02m');
    expect(formatDurationSeconds(90000)).toBe('1d 1h');
  });

  it('collapses a negative or non-finite span rather than rendering one', () => {
    // A clock-skewed snapshot must never produce "-4s ago".
    expect(formatDurationSeconds(-10)).toBe('0s');
    expect(formatDurationSeconds(Number.NaN)).toBe('0s');
  });

  it('keeps millisecond precision below a second', () => {
    expect(formatDurationMs(12)).toBe('12ms');
    expect(formatDurationMs(1500)).toBe('1s');
    expect(formatDurationMs(null)).toBeNull();
    expect(formatDurationMs(-1)).toBeNull();
  });
});

describe('clock-derived values', () => {
  const now = Date.parse('2026-08-01T12:00:00.000Z');

  it('measures elapsed and remaining time against the given instant', () => {
    expect(elapsedSeconds('2026-08-01T11:00:00.000Z', now)).toBe(3600);
    expect(remainingSeconds('2026-08-01T12:30:00.000Z', now)).toBe(1800);
  });

  it('clamps rather than reporting negative time', () => {
    expect(elapsedSeconds('2026-08-01T13:00:00.000Z', now)).toBe(0);
    expect(remainingSeconds('2026-08-01T11:00:00.000Z', now)).toBe(0);
  });

  it('answers null for an absent or unparseable instant', () => {
    expect(elapsedSeconds(null, now)).toBeNull();
    expect(remainingSeconds('not a date', now)).toBeNull();
  });
});

describe('lifetimeDisplay', () => {
  const now = Date.parse('2026-08-01T12:00:00.000Z');

  it('treats a null maximum as UNLIMITED, never as zero remaining', () => {
    expect(lifetimeDisplay(null, null, null, now)).toEqual({ kind: 'unlimited' });
  });

  it('prefers the live countdown over the snapshot figure', () => {
    // The snapshot said 7200s remaining; the expiry instant says 1800s.
    expect(lifetimeDisplay(7200, '2026-08-01T12:30:00.000Z', 7200, now)).toEqual({
      kind: 'bounded',
      maxSeconds: 7200,
      remaining: 1800,
      expiresAt: '2026-08-01T12:30:00.000Z',
    });
  });

  it('falls back to the snapshot figure when there is no expiry instant', () => {
    expect(lifetimeDisplay(7200, null, 600, now)).toMatchObject({ remaining: 600 });
  });

  it('reports an elapsed ceiling as zero remaining, which is NOT unlimited', () => {
    const display = lifetimeDisplay(7200, '2026-08-01T11:00:00.000Z', 0, now);
    expect(display).toMatchObject({ kind: 'bounded', remaining: 0 });
  });
});

describe('displayLocation', () => {
  it('passes a plain namespace through', () => {
    expect(displayLocation('chronoai-fkst')).toBe('chronoai-fkst');
  });

  it('can never emit anything that reads as a URL', () => {
    expect(displayLocation('https://sandbox.example/v1/x?token=abc')).toBe('sandbox.example');
    expect(displayLocation('https://user:pw@sandbox.example/v1')).toBe('sandbox.example');
    expect(displayLocation('sandbox.example:8443')).toBe('sandbox.example:8443');
  });

  it('answers null for an absent value', () => {
    expect(displayLocation(null)).toBeNull();
    expect(displayLocation('')).toBeNull();
  });
});

describe('safe-argument summaries', () => {
  it('is deterministic regardless of key order, so a poll cannot reflow a cell', () => {
    const a = summarizeArguments({ owner: 'acme', name: 'app', limit: 20 });
    const b = summarizeArguments({ limit: 20, name: 'app', owner: 'acme' });
    expect(a).toBe(b);
    expect(a).toBe('limit=20 name=app owner=acme');
  });

  it('reports container SIZE instead of container contents', () => {
    expect(summarizeValue([1, 2, 3])).toBe('[3]');
    expect(summarizeValue({ a: 1, b: 2 })).toBe('{2}');
    expect(summarizeValue(null)).toBe('null');
  });

  it('bounds a long value and the whole summary', () => {
    expect(summarizeValue('x'.repeat(200))).toHaveLength(32);
    const long = summarizeArguments(
      Object.fromEntries(Array.from({ length: 30 }, (_, i) => [`k${i}`, 'value']))
    );
    expect(long.length).toBeLessThanOrEqual(140);
    expect(long.endsWith('…')).toBe(true);
  });

  it('lists details fields in a stable order with containers as compact JSON', () => {
    expect(argumentEntries({ b: [1], a: 'x' })).toEqual([
      { key: 'a', value: 'x' },
      { key: 'b', value: '[1]' },
    ]);
  });

  it('renders an empty argument set as an empty summary, not "{}"', () => {
    expect(summarizeArguments({})).toBe('');
    expect(argumentEntries({})).toEqual([]);
  });
});

describe('tones', () => {
  it('maps each vocabulary onto a tone, defaulting to neutral', () => {
    expect(sandboxTone('running')).toBe('green');
    expect(sandboxTone('failed')).toBe('red');
    expect(sandboxTone('pending')).toBe('amber');
    expect(sandboxTone('unknown')).toBe('neutral');
    expect(outcomeTone('success')).toBe('green');
    expect(outcomeTone('server_error')).toBe('red');
    expect(outcomeTone('rejected')).toBe('amber');
    expect(deliveryTone('verified_in_posthog')).toBe('green');
    expect(deliveryTone('dead_letter')).toBe('red');
    expect(deliveryTone('queued')).toBe('amber');
    expect(deliveryTone('who-knows')).toBe('neutral');
  });
});

describe('cache keys', () => {
  it('changes with the identity generation', () => {
    const a = activityCacheKey(0, 'mine', DEFAULT_ACTIVITY_FILTERS);
    const b = activityCacheKey(1, 'mine', DEFAULT_ACTIVITY_FILTERS);
    expect(a).not.toBe(b);
  });

  it('changes with the scope, including the "server decides" state', () => {
    const explicit = activityCacheKey(0, 'mine', DEFAULT_ACTIVITY_FILTERS);
    const global = activityCacheKey(0, 'all', DEFAULT_ACTIVITY_FILTERS);
    const deferred = activityCacheKey(0, null, DEFAULT_ACTIVITY_FILTERS);
    expect(new Set([explicit, global, deferred]).size).toBe(3);
  });

  it('changes with every activity filter', () => {
    const base = activityCacheKey(0, 'all', DEFAULT_ACTIVITY_FILTERS);
    const variants = [
      { recordKind: 'all' as const },
      { preset: '7d' as const },
      { actorId: 7 },
      { actorLogin: 'alice' },
      { operationId: 'canvas_overview' },
      { method: 'GET' },
      { statusClass: '5xx' },
      { statusCode: 404 },
      { outcome: 'timeout' },
      { repoFullName: 'acme/app' },
      { triggerIssue: 42 },
      { sessionId: 'sess-1' },
      { requestId: 'req-1' },
    ];
    for (const variant of variants) {
      expect(activityCacheKey(0, 'all', { ...DEFAULT_ACTIVITY_FILTERS, ...variant })).not.toBe(base);
    }
  });

  it('ignores the resolved instants of a PRESET window, which slide by design', () => {
    const a = activityCacheKey(0, 'mine', { ...DEFAULT_ACTIVITY_FILTERS, from: 1, to: 2 });
    const b = activityCacheKey(0, 'mine', DEFAULT_ACTIVITY_FILTERS);
    expect(a).toBe(b);
  });

  it('includes the explicit bounds of a CUSTOM window', () => {
    const custom = { ...DEFAULT_ACTIVITY_FILTERS, preset: 'custom' as const, from: 1, to: 2 };
    expect(activityCacheKey(0, 'mine', custom)).not.toBe(
      activityCacheKey(0, 'mine', { ...custom, to: 3 })
    );
  });

  it('changes with every sandbox filter and never collides with an activity key', () => {
    const base = sandboxCacheKey(0, 'all', DEFAULT_SANDBOX_FILTERS);
    expect(base).not.toBe(activityCacheKey(0, 'all', DEFAULT_ACTIVITY_FILTERS));
    const variants = [
      { status: 'failed' },
      { backend: 'opensandbox' },
      { creatorId: 7 },
      { creatorLogin: 'alice' },
      { repoFullName: 'acme/app' },
      { sessionId: 'sess-1' },
      { triggerIssue: 42 },
      { attributionSource: 'unknown_legacy' },
    ];
    for (const variant of variants) {
      expect(sandboxCacheKey(0, 'all', { ...DEFAULT_SANDBOX_FILTERS, ...variant })).not.toBe(base);
    }
  });
});
