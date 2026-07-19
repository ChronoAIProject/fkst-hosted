import { describe, it, expect } from 'vitest';
import {
  formatAbsolute,
  formatIsoSgt,
  formatLocal,
  formatRelative,
  formatSgt,
} from './format';

// A fixed reference instant so relative buckets are deterministic regardless
// of when the suite runs.
const NOW = Date.UTC(2026, 6, 19, 12, 0, 0); // 2026-07-19T12:00:00Z

describe('formatLocal', () => {
  it('formats in the viewer local zone without an SGT suffix', () => {
    const s = formatLocal(NOW, 'en');
    // Mirrors the function contract (viewer-local, no fixed timezone).
    const ref = new Intl.DateTimeFormat('en-GB', {
      dateStyle: 'medium',
      timeStyle: 'short',
    }).format(new Date(NOW));
    expect(s).toBe(ref);
    expect(s).not.toContain('SGT');
  });

  it('accepts an ISO string as well as epoch-ms', () => {
    const iso = new Date(NOW).toISOString();
    expect(formatLocal(iso, 'en')).toBe(formatLocal(NOW, 'en'));
  });

  it('localizes to Chinese without changing the instant', () => {
    const zh = formatLocal(NOW, 'zh');
    const ref = new Intl.DateTimeFormat('zh-CN', {
      dateStyle: 'medium',
      timeStyle: 'short',
    }).format(new Date(NOW));
    expect(zh).toBe(ref);
  });

  it('returns the raw string for an unparseable ISO input', () => {
    expect(formatLocal('not-a-date', 'en')).toBe('not-a-date');
  });
});

describe('formatAbsolute', () => {
  it('carries the timezone name for a self-contained tooltip', () => {
    const abs = formatAbsolute(NOW, 'en');
    const ref = new Intl.DateTimeFormat('en-GB', {
      dateStyle: 'medium',
      timeStyle: 'long',
    }).format(new Date(NOW));
    expect(abs).toBe(ref);
    // 'long' timeStyle adds seconds + zone, so it strictly outsizes the terse
    // local render it backs.
    expect(abs.length).toBeGreaterThan(formatLocal(NOW, 'en').length);
  });

  it('falls back to the raw string when unparseable', () => {
    expect(formatAbsolute('nope', 'en')).toBe('nope');
  });
});

describe('formatRelative', () => {
  it('buckets a recent past event as "… ago"', () => {
    const twoMinAgo = NOW - 2 * 60_000;
    const s = formatRelative(twoMinAgo, 'en', NOW);
    expect(s.toLowerCase()).toContain('ago');
    expect(s).toContain('2');
  });

  it('buckets a near-future event as "in …"', () => {
    const inThreeHours = NOW + 3 * 3_600_000;
    const s = formatRelative(inThreeHours, 'en', NOW);
    expect(s.toLowerCase()).toContain('in');
    expect(s).toContain('3');
  });

  it('renders sub-minute deltas as the localized "just now"', () => {
    const justNow = new Intl.RelativeTimeFormat('en-GB', { numeric: 'auto' }).format(0, 'second');
    expect(formatRelative(NOW - 10_000, 'en', NOW)).toBe(justNow);
    expect(formatRelative(NOW, 'en', NOW)).toBe(justNow);
    expect(formatRelative(NOW + 10_000, 'en', NOW)).toBe(justNow);
  });

  it('picks the coarsest fitting unit (days) for larger gaps', () => {
    const threeDaysAgo = NOW - 3 * 86_400_000;
    const s = formatRelative(threeDaysAgo, 'en', NOW).toLowerCase();
    expect(s).toContain('3');
    expect(s).toContain('day');
  });

  it('localizes the relative phrase for Chinese', () => {
    const s = formatRelative(NOW - 2 * 60_000, 'zh', NOW);
    expect(s).toContain('前'); // "…前" == "… ago"
  });

  it('falls back to an absolute local render for an unparseable value', () => {
    expect(formatRelative('garbage', 'en', NOW)).toBe(formatLocal('garbage', 'en'));
  });
});

describe('formatSgt (deprecated, retained)', () => {
  it('still forces Singapore time with an SGT suffix', () => {
    // Epoch 0 is 1970-01-01 08:00 in SGT (UTC+8).
    const s = formatSgt(0, 'en');
    expect(s).toContain('SGT');
    expect(s).toContain('1970');
  });
});

describe('formatIsoSgt (deprecated, retained)', () => {
  it('formats a valid ISO string as SGT', () => {
    const s = formatIsoSgt('1970-01-01T00:00:00Z', 'en');
    expect(s).toContain('SGT');
  });

  it('returns null for null or unparseable input', () => {
    expect(formatIsoSgt(null, 'en')).toBeNull();
    expect(formatIsoSgt('not-a-date', 'en')).toBeNull();
  });
});
