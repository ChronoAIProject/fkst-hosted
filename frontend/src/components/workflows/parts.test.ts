import { describe, expect, it } from 'vitest';
import { formatDuration, relativeTo } from './parts';

// The two pure formatters are the ONLY place this surface does arithmetic on a
// time, and the rule they live under is narrow: they may render a distance the
// API already committed to, never compute a firing. These cases pin the
// boundaries where a coarse renderer is easy to get subtly wrong.

const t = {
  inDays: 'in {d}d',
  inHours: 'in {h}h',
  inMinutes: 'in {m}m',
  imminent: 'due now',
  overdue: 'overdue',
  never: '—',
};

const NOW = Date.parse('2026-08-05T12:00:00Z');
const inSeconds = (seconds: number) => new Date(NOW + seconds * 1000).toISOString();

describe('relativeTo', () => {
  it('renders an absent or unparseable instant as absent, not as a broken distance', () => {
    expect(relativeTo(null, NOW, t)).toBe('—');
    expect(relativeTo('not a timestamp', NOW, t)).toBe('—');
  });

  it('calls a slot that has passed without a run overdue rather than a negative distance', () => {
    // The clock is late; saying so is more honest than "in -3 minutes".
    expect(relativeTo(inSeconds(-3600), NOW, t)).toBe('overdue');
  });

  it('treats the minute around now as imminent, in both directions', () => {
    // A firing seconds in the past is not yet evidence of a late clock.
    expect(relativeTo(inSeconds(-30), NOW, t)).toBe('due now');
    expect(relativeTo(inSeconds(30), NOW, t)).toBe('due now');
  });

  it('steps up through minutes, hours and days', () => {
    expect(relativeTo(inSeconds(20 * 60), NOW, t)).toBe('in 20m');
    expect(relativeTo(inSeconds(5 * 3600), NOW, t)).toBe('in 5h');
    expect(relativeTo(inSeconds(5 * 86400), NOW, t)).toBe('in 5d');
  });

  it('stays in hours up to two days, where a day count would round away the useful part', () => {
    expect(relativeTo(inSeconds(47 * 3600), NOW, t)).toBe('in 47h');
    expect(relativeTo(inSeconds(48 * 3600), NOW, t)).toBe('in 2d');
  });
});

describe('formatDuration', () => {
  it('renders an absent or negative duration as the caller’s absent label', () => {
    expect(formatDuration(null, '—')).toBe('—');
    expect(formatDuration(-1, '—')).toBe('—');
  });

  it('renders zero as a real duration, not as absent', () => {
    // A step that finished within the same second ran; it is not missing.
    expect(formatDuration(0, '—')).toBe('0s');
  });

  it('steps up through seconds, minutes and hours', () => {
    expect(formatDuration(42, '—')).toBe('42s');
    expect(formatDuration(185, '—')).toBe('3m 5s');
    expect(formatDuration(3720, '—')).toBe('1h 2m');
  });
});
