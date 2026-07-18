import { describe, it, expect, vi } from 'vitest';
// Node clicks use fireEvent (a lone `click`, no mousedown): user-event's full
// pointer sequence trips d3-zoom's pane listener, which dereferences
// `event.view.document` — null on jsdom-synthesized MouseEvents.
import { fireEvent, render, screen } from '@testing-library/react';
import { CanvasFlow, buildNodes } from './flow';
import type { AccountOverview, RepoOverview } from '@/lib/api/types';

let nextId = 1;
const repo = (over: Partial<RepoOverview> & Pick<RepoOverview, 'name'>): RepoOverview => ({
  id: nextId++,
  owner: 'shining',
  private: false,
  admin: true,
  installed: false,
  active_sessions: 0,
  packages: [],
  ...over,
});

const account = (
  over: Partial<AccountOverview> & Pick<AccountOverview, 'login'>
): AccountOverview => ({
  kind: 'personal',
  owner: true,
  installed: false,
  installation_id: null,
  repository_selection: null,
  counts_complete: true,
  repos: [],
  ...over,
});

const noop = () => {};

function renderFlow(props: Partial<Parameters<typeof CanvasFlow>[0]> = {}) {
  return render(
    <div style={{ width: '800px', height: '600px' }}>
      <CanvasFlow
        level={{ kind: 'root' }}
        accounts={[]}
        repos={[]}
        repoSessions={null}
        repoInstalled={false}
        onOpenAccount={noop}
        onOpenRepo={noop}
        {...props}
      />
    </div>
  );
}

describe('buildNodes', () => {
  it('produces one account node per account at root, positioned on the grid', () => {
    const nodes = buildNodes({
      level: { kind: 'root' },
      accounts: [account({ login: 'shining' }), account({ login: 'acme', kind: 'org' })],
      repos: [],
      repoSessions: null,
      repoInstalled: false,
      onOpenAccount: noop,
      onOpenRepo: noop,
    });
    expect(nodes.map((n) => n.id)).toEqual(['account:shining', 'account:acme']);
    expect(nodes.every((n) => n.type === 'account')).toBe(true);
    expect(nodes[0]!.position).toEqual({ x: 0, y: 0 });
    expect(nodes[1]!.position.x).toBeGreaterThan(0);
  });

  it('produces repo nodes at level 1 and a single detail node at level 2', () => {
    const repoNodes = buildNodes({
      level: { kind: 'account', login: 'shining' },
      accounts: [],
      repos: [repo({ name: 'lab' }), repo({ name: 'rocket' })],
      repoSessions: null,
      repoInstalled: false,
      onOpenAccount: noop,
      onOpenRepo: noop,
    });
    expect(repoNodes.map((n) => n.id)).toEqual(['repo:shining/lab', 'repo:shining/rocket']);

    const detail = buildNodes({
      level: { kind: 'repo', owner: 'shining', name: 'lab' },
      accounts: [],
      repos: [],
      repoSessions: null,
      repoInstalled: true,
      onOpenAccount: noop,
      onOpenRepo: noop,
    });
    expect(detail).toHaveLength(1);
    expect(detail[0]!.type).toBe('repoDetail');
  });
});

describe('CanvasFlow', () => {
  it('renders account cards with status classes, repo dots and owner badge', () => {
    renderFlow({
      accounts: [
        account({
          login: 'shining',
          installed: true,
          repos: [
            repo({ name: 'busy', installed: true, active_sessions: 2 }),
            repo({ name: 'quiet', installed: true }),
            repo({ name: 'bare' }),
          ],
        }),
        account({ login: 'acme', kind: 'org', owner: false }),
      ],
    });

    // Both cards render as keyboard-reachable buttons with aria labels.
    const open = screen.getByRole('button', { name: 'Open account shining' });
    expect(open).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Open account acme' })).toBeInTheDocument();

    // Active account: blinking glow class + textual active badge (motion-free cue).
    expect(open.className).toContain('anim-node-glow');
    expect(screen.getByText('2 active')).toBeInTheDocument();

    // Repo dots inside the card carry per-repo status.
    const dots = open.querySelectorAll('[data-status]');
    expect([...dots].map((d) => d.getAttribute('data-status'))).toEqual([
      'active',
      'installed',
      'none',
    ]);

    // Owner badge only on accounts the viewer owns/admins.
    expect(screen.getAllByText('owner')).toHaveLength(1);

    // The org account has no App: grey status text, no glow.
    const org = screen.getByRole('button', { name: 'Open account acme' });
    expect(org.className).not.toContain('anim-node-glow');
    expect(screen.getByText('no App')).toBeInTheDocument();
  });

  it('opens an account on click', () => {
    const onOpenAccount = vi.fn();
    renderFlow({ accounts: [account({ login: 'shining' })], onOpenAccount });
    fireEvent.click(screen.getByRole('button', { name: 'Open account shining' }));
    expect(onOpenAccount).toHaveBeenCalledWith('shining');
  });

  it('renders repo cards at level 1 and opens one on click', () => {
    const onOpenRepo = vi.fn();
    renderFlow({
      level: { kind: 'account', login: 'shining' },
      repos: [
        repo({ name: 'lab', private: true, installed: true, active_sessions: 1 }),
        repo({ name: 'bare' }),
      ],
      onOpenRepo,
    });

    expect(screen.getByText('private')).toBeInTheDocument();
    expect(screen.getByText('1 active')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Open repository shining/bare' }));
    expect(onOpenRepo).toHaveBeenCalledWith('shining', 'bare');
  });

  it('renders the level-2 detail node with a session summary', () => {
    renderFlow({
      level: { kind: 'repo', owner: 'shining', name: 'lab' },
      repoInstalled: true,
      repoSessions: {
        owner: 'shining',
        name: 'lab',
        installed: true,
        sessions: [
          {
            session_id: 'abc',
            name: 'nightly',
            work_label: 'fkst-work',
            auto_merge: true,
            environment: null,
            packages: [],
            invalid_reason: null,
            status_labels: [],
            trigger: {
              number: 7,
              title: 'nightly',
              state: 'open',
              author: 'shining',
              labels: [],
              html_url: 'https://github.com/shining/lab/issues/7',
              created_at: '2026-07-01T00:00:00Z',
              updated_at: '2026-07-02T00:00:00Z',
              closed_at: null,
            },
            work_issues: [],
            log_url: null,
            liveness: 'live',
            prs: [],
          },
        ],
      },
    });

    expect(screen.getByText('shining/lab')).toBeInTheDocument();
    expect(screen.getByText('nightly')).toBeInTheDocument();
    expect(screen.getByText('#7')).toBeInTheDocument();
  });

  it('shows an in-node skeleton while level-2 data loads', () => {
    renderFlow({
      level: { kind: 'repo', owner: 'shining', name: 'lab' },
      repoInstalled: true,
      repoSessions: null,
    });
    expect(screen.getByLabelText('Loading details…')).toBeInTheDocument();
  });
});
