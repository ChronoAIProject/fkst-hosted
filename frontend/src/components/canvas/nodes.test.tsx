import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import type { NodeProps } from '@xyflow/react';
import { AccountNode, RepoNode, RepoDetailNode } from './nodes';
import type {
  AccountFlowNode,
  RepoFlowNode,
  DetailFlowNode,
  AccountNodeData,
  RepoNodeData,
  DetailNodeData,
} from './nodes';
import type { AccountOverview, RepoOverview, SessionDetail } from '@/lib/api/types';

// The node components only read `data` off NodeProps, so a minimal cast lets us
// render each one in isolation (outside the React Flow store) — useContent()
// defaults to English without a LanguageProvider.
const renderAccount = (data: AccountNodeData) =>
  render(<AccountNode {...({ data } as unknown as NodeProps<AccountFlowNode>)} />);
const renderRepo = (data: RepoNodeData) =>
  render(<RepoNode {...({ data } as unknown as NodeProps<RepoFlowNode>)} />);
const renderDetail = (data: DetailNodeData) =>
  render(<RepoDetailNode {...({ data } as unknown as NodeProps<DetailFlowNode>)} />);

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

const session = (over: Partial<SessionDetail> = {}): SessionDetail => ({
  session_id: `s${nextId++}`,
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
  ...over,
});

const noop = () => {};

describe('AccountNode', () => {
  it('renders login, active badge, repo dots, owner badge and mount stagger', () => {
    renderAccount({
      account: account({
        login: 'shining',
        installed: true,
        repos: [
          repo({ name: 'busy', installed: true, active_sessions: 2 }),
          repo({ name: 'quiet', installed: true }),
          repo({ name: 'bare' }),
        ],
      }),
      onOpen: noop,
      index: 2,
    });

    const btn = screen.getByRole('button', { name: 'Open account shining' });
    // Active account: blinking glow + motion-free textual count.
    expect(btn.className).toContain('anim-node-glow');
    expect(screen.getByText('2 active')).toBeInTheDocument();

    // Per-repo dots carry their own status.
    const dots = btn.querySelectorAll('[data-status]');
    expect([...dots].map((d) => d.getAttribute('data-status'))).toEqual([
      'active',
      'installed',
      'none',
    ]);
    expect(screen.getByText('owner')).toBeInTheDocument();

    // Body-level mount stagger: anim-row-in + index-derived --stagger delay.
    expect(btn.className).toContain('anim-row-in');
    expect(btn.style.getPropertyValue('--stagger')).toBe('80ms'); // index 2 * 40ms
  });

  it('defaults to a zero stagger delay when no index is supplied', () => {
    renderAccount({ account: account({ login: 'acme', kind: 'org', owner: false }), onOpen: noop });
    const btn = screen.getByRole('button', { name: 'Open account acme' });
    // No App → grey status text, no glow; stagger degrades to instant.
    expect(btn.className).not.toContain('anim-node-glow');
    expect(screen.getByText('no App')).toBeInTheDocument();
    expect(btn.style.getPropertyValue('--stagger')).toBe('0ms');
  });

  it('opens the account on click', () => {
    const onOpen = vi.fn();
    renderAccount({ account: account({ login: 'shining' }), onOpen });
    fireEvent.click(screen.getByRole('button', { name: 'Open account shining' }));
    expect(onOpen).toHaveBeenCalledWith('shining');
  });
});

describe('RepoNode', () => {
  it('renders name, visibility, active badge, package count and mount stagger', () => {
    renderRepo({
      repo: repo({
        name: 'lab',
        private: true,
        installed: true,
        active_sessions: 1,
        packages: ['a/b@ref:base'],
      }),
      onOpen: noop,
      index: 3,
    });

    const btn = screen.getByRole('button', { name: 'Open repository shining/lab' });
    expect(screen.getByText('private')).toBeInTheDocument();
    expect(screen.getByText('1 active')).toBeInTheDocument();
    expect(btn.className).toContain('anim-row-in');
    expect(btn.style.getPropertyValue('--stagger')).toBe('120ms'); // index 3 * 40ms
  });

  it('opens the repository on click', () => {
    const onOpen = vi.fn();
    renderRepo({ repo: repo({ name: 'bare' }), onOpen });
    fireEvent.click(screen.getByRole('button', { name: 'Open repository shining/bare' }));
    expect(onOpen).toHaveBeenCalledWith('shining', 'bare');
  });
});

describe('RepoDetailNode', () => {
  it('shows the shimmer skeleton while the level-2 fetch is in flight', () => {
    renderDetail({ owner: 'shining', name: 'lab', installed: true, sessions: null });
    expect(screen.getByText('shining/lab')).toBeInTheDocument();
    expect(screen.getByLabelText('Loading details…')).toBeInTheDocument();
  });

  it('renders a short "could not load" line on a failed fetch instead of shimmering', () => {
    renderDetail({
      owner: 'shining',
      name: 'lab',
      installed: true,
      sessions: null,
      sessionsFailed: true,
    });
    // Failed is terminal: the load message shows, the skeleton does NOT.
    expect(
      screen.getByText('Could not load the sessions of this repository. Please try again.')
    ).toBeInTheDocument();
    expect(screen.queryByLabelText('Loading details…')).toBeNull();
  });

  it('shows the empty state when the repo has no sessions', () => {
    renderDetail({
      owner: 'shining',
      name: 'lab',
      installed: true,
      sessions: { owner: 'shining', name: 'lab', installed: true, sessions: [] },
    });
    expect(screen.getByText('No fkst sessions in this repository.')).toBeInTheDocument();
  });

  it('renders a compact status summary plus the session list when loaded', () => {
    renderDetail({
      owner: 'shining',
      name: 'lab',
      installed: true,
      sessions: {
        owner: 'shining',
        name: 'lab',
        installed: true,
        sessions: [
          session({ session_id: 'a', name: 'nightly', trigger: { ...session().trigger, number: 7 } }),
          // A closed trigger is not "active" → summary counts 1 of 2 active.
          session({
            session_id: 'b',
            name: 'weekly',
            trigger: { ...session().trigger, number: 8, state: 'closed', closed_at: '2026-07-03T00:00:00Z' },
          }),
        ],
      },
    });

    // Summary: active-count badge + total.
    expect(screen.getByText('1 active')).toBeInTheDocument();
    expect(screen.getByText('Sessions · 2')).toBeInTheDocument();
    // The compact list keeps per-session name + trigger number.
    expect(screen.getByText('nightly')).toBeInTheDocument();
    expect(screen.getByText('#7')).toBeInTheDocument();
    expect(screen.getByText('weekly')).toBeInTheDocument();
    expect(screen.getByText('#8')).toBeInTheDocument();
  });

  it('falls back to the trigger name and renders sessions lacking a session_id (B2 key)', () => {
    // A session with no session_id exercises the `t-${number}` fallback key —
    // which, post-B2, no longer carries the positional `-${i}` churn suffix.
    renderDetail({
      owner: 'shining',
      name: 'lab',
      installed: true,
      sessions: {
        owner: 'shining',
        name: 'lab',
        installed: true,
        sessions: [session({ session_id: null, name: null, trigger: { ...session().trigger, number: 9 } })],
      },
    });
    // Null name → the invalid-trigger placeholder still renders.
    expect(screen.getByText('#9')).toBeInTheDocument();
    expect(screen.getByText('Invalid trigger')).toBeInTheDocument();
  });
});
