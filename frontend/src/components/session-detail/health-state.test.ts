import { describe, it, expect } from 'vitest';
import type { HealthStatus, SessionHealth, StalenessState } from '@/lib/api/health';
import { HEALTH_TONE, healthChip, minutes, showsStaleNotice } from './health-state';

function health(state: StalenessState, status?: HealthStatus, ageSecs = 120): SessionHealth {
  const latest = status
    ? {
        id: 'r-1',
        generated_at: '2026-07-30T14:15:00Z',
        status,
        status_raw: status,
        headline: 'a headline',
        producer: 'fkst-health@0.1.0',
      }
    : null;
  return {
    session_id: 'sess-1',
    reports: latest ? [latest] : [],
    latest,
    staleness: { state, expected_interval_secs: 600, age_secs: latest ? ageSecs : null },
  };
}

describe('HEALTH_TONE', () => {
  it('maps every v1 status to a Chip tone', () => {
    expect(HEALTH_TONE).toEqual({
      working: 'green',
      idle: 'neutral',
      blocked: 'amber',
      stalled: 'red',
      failing: 'red',
      unknown: 'neutral',
    });
  });
});

describe('healthChip', () => {
  it.each([
    ['working', 'green'],
    ['idle', 'neutral'],
    ['blocked', 'amber'],
    ['stalled', 'red'],
    ['failing', 'red'],
    ['unknown', 'neutral'],
  ] as Array<[HealthStatus, string]>)('renders %s in its mapped tone', (status, tone) => {
    expect(healthChip(health('fresh', status))).toEqual({ kind: 'status', status, tone });
  });

  it('lets a STALE heartbeat override the tone even when the last status was working', () => {
    // A 35-minute-old "working" verdict is not evidence that the session is
    // working; rendering it green would actively mislead.
    expect(healthChip(health('stale', 'working'))).toEqual({ kind: 'stale', tone: 'amber' });
  });

  it('renders not_running NEUTRALLY and never as the stale chip', () => {
    // THE false-alarm regression: a reaped pod is the normal end of a session's
    // work, so it must never look like a fault.
    const chip = healthChip(health('not_running', 'working', 99_999));
    expect(chip).toEqual({ kind: 'status', status: 'working', tone: 'neutral' });
    expect(chip?.kind).not.toBe('stale');
  });

  it('renders NO chip at all when nothing has been reported yet', () => {
    expect(healthChip(health('never_reported'))).toBeNull();
  });

  it('renders no chip for a not_running session that never reported', () => {
    expect(healthChip(health('not_running'))).toBeNull();
  });

  it('renders no chip before the listing has loaded', () => {
    expect(healthChip(null)).toBeNull();
  });

  it('falls back to the first report when latest is absent', () => {
    const withoutLatest = { ...health('fresh', 'blocked'), latest: null };
    expect(healthChip(withoutLatest)).toEqual({
      kind: 'status',
      status: 'blocked',
      tone: 'amber',
    });
  });
});

describe('showsStaleNotice', () => {
  it('is true ONLY for stale', () => {
    expect(showsStaleNotice(health('stale', 'working'))).toBe(true);
    expect(showsStaleNotice(health('not_running', 'working', 99_999))).toBe(false);
    expect(showsStaleNotice(health('fresh', 'working'))).toBe(false);
    expect(showsStaleNotice(health('never_reported'))).toBe(false);
    expect(showsStaleNotice(null)).toBe(false);
  });
});

describe('minutes', () => {
  it('floors seconds to whole minutes', () => {
    expect(minutes(0)).toBe(0);
    expect(minutes(59)).toBe(0);
    expect(minutes(600)).toBe(10);
    expect(minutes(2100)).toBe(35);
  });

  it('returns null for an absent or nonsensical value', () => {
    expect(minutes(null)).toBeNull();
    expect(minutes(undefined)).toBeNull();
    expect(minutes(-1)).toBeNull();
    expect(minutes(Number.NaN)).toBeNull();
  });
});
