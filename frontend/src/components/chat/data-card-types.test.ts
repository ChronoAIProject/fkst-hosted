import { describe, expect, it } from 'vitest';
import { parseDataCard } from './data-card-types';

/**
 * The parser's job is to be strict without being brittle: a card whose ROWS are
 * malformed is dropped entirely (a half-empty card reads as "there is nothing there"),
 * while a missing count is tolerated because it only costs a footnote.
 */
describe('parseDataCard', () => {
  it('rejects anything that is not a card', () => {
    for (const value of [null, undefined, 'card', 42, [], {}]) {
      expect(parseDataCard(value)).toBeNull();
    }
  });

  it('rejects an unrecognized kind so a newer server degrades to no card', () => {
    expect(parseDataCard({ kind: 'quantum_telemetry', rows: [] })).toBeNull();
  });

  // ---- environments ------------------------------------------------------

  it('parses an environments card with its counts', () => {
    const card = parseDataCard({
      kind: 'environments',
      profiles: [
        {
          name: 'video-studio',
          status: 'ready',
          validated_at: '2026-07-18T21:57:56Z',
          install_command_count: 2,
          variable_count: 2,
          secret_count: 1,
        },
      ],
      omitted: 3,
    });
    expect(card).toEqual({
      kind: 'environments',
      profiles: [
        {
          name: 'video-studio',
          status: 'ready',
          validated_at: '2026-07-18T21:57:56Z',
          install_command_count: 2,
          variable_count: 2,
          secret_count: 1,
        },
      ],
      omitted: 3,
    });
  });

  it('keeps an empty environments card, because "you have none" is an answer', () => {
    expect(parseDataCard({ kind: 'environments', profiles: [] })).toEqual({
      kind: 'environments',
      profiles: [],
      omitted: 0,
    });
  });

  it('drops an environments card whose rows are malformed', () => {
    expect(parseDataCard({ kind: 'environments', profiles: [{ status: 'ready' }] })).toBeNull();
    expect(parseDataCard({ kind: 'environments', profiles: 'nope' })).toBeNull();
  });

  it('tolerates a missing count rather than losing the rows', () => {
    const card = parseDataCard({
      kind: 'environments',
      profiles: [{ name: 'a' }],
      omitted: 'lots',
    });
    expect(card).toMatchObject({ omitted: 0 });
    expect(card).toMatchObject({ profiles: [{ name: 'a', status: '', install_command_count: 0 }] });
  });

  // ---- environment detail ------------------------------------------------

  it('parses an environment detail with secret NAMES only', () => {
    const card = parseDataCard({
      kind: 'environment_detail',
      name: 'video-studio',
      status: 'ready',
      validated_at: '2026-07-18T21:57:56Z',
      install: ['apt-get install -y ffmpeg'],
      variables: [{ key: 'FFMPEG_PRESET', value: 'veryfast' }],
      secret_keys: ['YT_API_KEY'],
    });
    expect(card).toMatchObject({
      kind: 'environment_detail',
      install: ['apt-get install -y ffmpeg'],
      secret_keys: ['YT_API_KEY'],
    });
    // The union has no field a secret VALUE could occupy.
    expect(JSON.stringify(card)).not.toContain('secrets');
  });

  it('drops an environment detail missing its required lists', () => {
    expect(parseDataCard({ kind: 'environment_detail', name: 'x' })).toBeNull();
    expect(
      parseDataCard({ kind: 'environment_detail', name: 'x', install: [], secret_keys: [] })
    ).toBeNull();
  });

  // ---- outcomes ----------------------------------------------------------

  it('parses outcomes and normalizes a missing work issue to null', () => {
    const card = parseDataCard({
      kind: 'outcomes',
      owner: 'acme',
      name: 'site',
      trigger_issue: 12,
      pull_requests: [
        {
          number: 20,
          title: 'Add the hero',
          html_url: 'https://x/20',
          state: 'closed',
          merged: true,
          work_issue: 15,
          files_changed: 2,
        },
        { number: 21, title: 'Open one', html_url: 'https://x/21', state: 'open' },
      ],
      merged: 1,
      omitted: 0,
    });
    expect(card).toMatchObject({ kind: 'outcomes', merged: 1 });
    const outcomes = card as Extract<ReturnType<typeof parseDataCard>, { kind: 'outcomes' }>;
    expect(outcomes.pull_requests[1]).toMatchObject({ work_issue: null, merged: false });
  });

  it('drops outcomes without repository coordinates', () => {
    expect(parseDataCard({ kind: 'outcomes', owner: 'acme', pull_requests: [] })).toBeNull();
  });

  // ---- logs --------------------------------------------------------------

  it('keeps a live run distinguishable from a finished one', () => {
    const card = parseDataCard({
      kind: 'log_runs',
      session_id: 's1',
      runs: [
        { run_id: 'a', started_at: 't0', ended_at: 't1' },
        { run_id: 'b', started_at: 't2' },
      ],
    });
    const runs = card as Extract<ReturnType<typeof parseDataCard>, { kind: 'log_runs' }>;
    expect(runs.runs[0]!.ended_at).toBe('t1');
    // null, not '' — the card renders "RUNNING" off exactly this.
    expect(runs.runs[1]!.ended_at).toBeNull();
  });

  it('parses a log manifest and defaults an unknown size to zero', () => {
    const card = parseDataCard({
      kind: 'log_manifest',
      session_id: 's1',
      run: 'run-1',
      files: [{ path: 'codex/run.log' }],
    });
    expect(card).toMatchObject({
      kind: 'log_manifest',
      run: 'run-1',
      files: [{ path: 'codex/run.log', size_bytes: 0 }],
    });
  });

  it('drops a log card without its session id', () => {
    expect(parseDataCard({ kind: 'log_runs', runs: [] })).toBeNull();
    expect(parseDataCard({ kind: 'log_manifest', files: [] })).toBeNull();
  });
});
