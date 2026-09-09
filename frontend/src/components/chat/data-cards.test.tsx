import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { LanguageProvider } from '@/i18n';
import { DataCards } from './data-cards';
import type { DataCard } from './data-card-types';

function renderCards(cards: DataCard[]) {
  return render(
    <LanguageProvider>
      <DataCards cards={cards} />
    </LanguageProvider>
  );
}

describe('DataCards', () => {
  it('renders nothing when a turn produced no cards', () => {
    const { container } = renderCards([]);
    expect(container.firstChild).toBeNull();
  });

  it('shows each environment with its status and counts', () => {
    renderCards([
      {
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
        omitted: 0,
      },
    ]);
    expect(screen.getByTestId('chat-card-environments')).toBeTruthy();
    expect(screen.getByText('video-studio')).toBeTruthy();
    // The chip TEXT is the raw status, so meaning never rides on colour alone.
    expect(screen.getByText('ready')).toBeTruthy();
    expect(screen.getByText(/2 install/)).toBeTruthy();
  });

  it('says so when there are no environments rather than rendering an empty frame', () => {
    renderCards([{ kind: 'environments', profiles: [], omitted: 0 }]);
    expect(screen.getByText(/No saved environment profiles/i)).toBeTruthy();
  });

  it('reports how many rows it did not show', () => {
    renderCards([
      { kind: 'environments', profiles: [{ name: 'a', status: 'ready', validated_at: '', install_command_count: 0, variable_count: 0, secret_count: 0 }], omitted: 7 },
    ]);
    // Silent truncation would read as "that is all of them".
    expect(screen.getByTestId('chat-card-omitted').textContent).toContain('7');
  });

  it('masks every secret value on an environment detail', () => {
    renderCards([
      {
        kind: 'environment_detail',
        name: 'video-studio',
        status: 'ready',
        validated_at: '2026-07-18T21:57:56Z',
        install: ['apt-get install -y ffmpeg'],
        variables: [{ key: 'FFMPEG_PRESET', value: 'veryfast' }],
        secret_keys: ['YT_API_KEY'],
      },
    ]);
    const card = screen.getByTestId('chat-card-environment-detail');
    expect(card.textContent).toContain('apt-get install -y ffmpeg');
    expect(card.textContent).toContain('FFMPEG_PRESET');
    expect(card.textContent).toContain('YT_API_KEY');
    // A fixed mask, never anything derived from a real value.
    expect(card.textContent).toContain('••••');
  });

  it('links each pull request and marks the merged ones', () => {
    renderCards([
      {
        kind: 'outcomes',
        owner: 'acme',
        name: 'site',
        trigger_issue: 12,
        pull_requests: [
          {
            number: 20,
            title: 'Add the hero',
            html_url: 'https://github.com/acme/site/pull/20',
            state: 'closed',
            merged: true,
            work_issue: 15,
            files_changed: 2,
          },
        ],
        merged: 1,
        omitted: 0,
      },
    ]);
    const link = screen.getByTestId('chat-card-pr-link') as HTMLAnchorElement;
    expect(link.href).toBe('https://github.com/acme/site/pull/20');
    // A link built from server data must not hand the opened page a window handle.
    expect(link.rel).toContain('noopener');
    expect(screen.getByText('MERGED')).toBeTruthy();
  });

  it('distinguishes a live run from a finished one', () => {
    renderCards([
      {
        kind: 'log_runs',
        session_id: 'sess-1',
        runs: [
          { run_id: 'run-a', started_at: '2026-07-18T21:57:00Z', ended_at: '2026-07-18T22:10:00Z' },
          { run_id: 'run-b', started_at: '2026-07-19T09:00:00Z', ended_at: null },
        ],
        omitted: 0,
      },
    ]);
    const card = screen.getByTestId('chat-card-log-runs');
    expect(card.textContent).toContain('run-a');
    // The live run says RUNNING instead of leaving an empty column.
    expect(card.textContent).toContain('RUNNING');
  });

  it('renders log file sizes in human units', () => {
    renderCards([
      {
        kind: 'log_manifest',
        session_id: 'sess-1',
        run: 'run-a',
        files: [
          { path: 'codex/run.log', size_bytes: 4096 },
          { path: 'codex/big.log', size_bytes: 5 * 1024 * 1024 },
          { path: 'codex/tiny.log', size_bytes: 12 },
        ],
        omitted: 0,
      },
    ]);
    const card = screen.getByTestId('chat-card-log-manifest');
    expect(card.textContent).toContain('4.0 KB');
    expect(card.textContent).toContain('5.0 MB');
    expect(card.textContent).toContain('12 B');
  });

  it('renders several cards from one turn in arrival order', () => {
    renderCards([
      { kind: 'environments', profiles: [], omitted: 0 },
      { kind: 'log_runs', session_id: 's', runs: [], omitted: 0 },
    ]);
    const cards = screen.getByTestId('chat-data-cards');
    expect(cards.children.length).toBe(2);
  });
});
