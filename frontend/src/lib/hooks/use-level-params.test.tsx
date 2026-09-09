import { describe, expect, it, beforeEach, afterEach } from 'vitest';
import { act, render } from '@testing-library/react';
import { BrowserRouter } from 'react-router-dom';
import { useLevelParams } from './use-level-params';
import type { CanvasLevel } from '@/components/canvas/level';
import type { AccountOverview, OverviewResponse } from '@/lib/api/types';

/** Expose the hook's surface to a test through a probe component. */
type Hook = ReturnType<typeof useLevelParams>;

function renderHook(search: string) {
  window.history.replaceState(null, '', `/dashboard${search}`);
  const captured: { current: Hook | null } = { current: null };
  function Probe() {
    captured.current = useLevelParams();
    return null;
  }
  const utils = render(
    <BrowserRouter>
      <Probe />
    </BrowserRouter>
  );
  return { ...utils, hook: () => captured.current!, captured };
}

const overview = (accounts: AccountOverview[]): OverviewResponse =>
  ({
    app_slug: 'app',
    viewer: { login: 'shining' },
    global_admin: false,
    accounts,
    totals: { sessions: 0, packages: [] },
    broader_oauth_available: false,
  }) as OverviewResponse;

const account = (login: string, repos: string[]): AccountOverview =>
  ({
    login,
    kind: 'personal',
    owner: true,
    installed: true,
    installation_id: 1,
    repository_selection: 'all',
    counts_complete: true,
    repos: repos.map((name) => ({ owner: login, name })),
  }) as unknown as AccountOverview;

describe('useLevelParams', () => {
  beforeEach(() => {
    window.history.replaceState(null, '', '/dashboard');
  });
  afterEach(() => {
    window.history.replaceState(null, '', '/dashboard');
  });

  it('reads the initial level from the URL', () => {
    const { hook } = renderHook('?owner=acme&repo=site&session=sess-1');
    expect(hook().initial.level).toEqual({ kind: 'repo', owner: 'acme', name: 'site' });
    expect(hook().initial.sessionKey).toBe('sess-1');
  });

  it('reports root when the URL carries nothing', () => {
    const { hook } = renderHook('');
    expect(hook().initial.level).toEqual({ kind: 'root' });
    expect(hook().initial.sessionKey).toBeUndefined();
  });

  it('keeps the initial read stable across re-renders', () => {
    // The dashboard polls; a re-render that re-parsed the URL would yank the user
    // back to wherever they started.
    const { hook, rerender } = renderHook('?owner=acme');
    const first = hook().initial;
    act(() => {
      hook().navigateLevel({ kind: 'root' });
    });
    rerender(
      <BrowserRouter>
        <div />
      </BrowserRouter>
    );
    expect(hook().initial).toBe(first);
  });

  it('writes a level into the URL without pushing history', () => {
    const { hook } = renderHook('');
    const before = window.history.length;
    act(() => {
      hook().navigateLevel({ kind: 'repo', owner: 'acme', name: 'site' }, 'sess-1');
    });
    expect(window.location.search).toBe('?owner=acme&repo=site&session=sess-1');
    // Browsing zoom levels must not fill the back stack.
    expect(window.history.length).toBe(before);
  });

  it('clears every parameter', () => {
    const { hook } = renderHook('?owner=acme&repo=site&session=sess-1');
    act(() => {
      hook().clearParams();
    });
    expect(window.location.search).toBe('');
  });

  describe('isUnknownLocation', () => {
    const known = overview([account('acme', ['site', 'docs'])]);

    it('accepts a level that exists', () => {
      const { hook } = renderHook('');
      expect(hook().isUnknownLocation(known, { kind: 'account', login: 'acme' })).toBe(false);
    });

    it('accepts a repo that exists', () => {
      const { hook } = renderHook('');
      expect(
        hook().isUnknownLocation(known, { kind: 'repo', owner: 'acme', name: 'site' })
      ).toBe(false);
    });

    it('rejects an unknown owner', () => {
      const { hook } = renderHook('');
      expect(hook().isUnknownLocation(known, { kind: 'account', login: 'ghost' })).toBe(true);
    });

    it('rejects a known owner with an unknown repo', () => {
      const { hook } = renderHook('');
      expect(
        hook().isUnknownLocation(known, { kind: 'repo', owner: 'acme', name: 'ghost' })
      ).toBe(true);
    });

    it('matches case-insensitively', () => {
      // GitHub logins and repo names are case-insensitive, so a differently-cased
      // pasted URL must still land in the right place.
      const { hook } = renderHook('');
      expect(
        hook().isUnknownLocation(known, { kind: 'repo', owner: 'ACME', name: 'SITE' })
      ).toBe(false);
    });

    it('never rejects the root level', () => {
      const { hook } = renderHook('');
      expect(hook().isUnknownLocation(known, { kind: 'root' })).toBe(false);
    });

    it('waits for the overview rather than guessing', () => {
      const { hook } = renderHook('');
      expect(hook().isUnknownLocation(null, { kind: 'account', login: 'ghost' })).toBe(false);
    });

    it('checks only once, so a later poll cannot fight the user', () => {
      // A refetch that transiently drops an account must not yank the user out of
      // the level they navigated to themselves.
      const { hook } = renderHook('');
      const ghost: CanvasLevel = { kind: 'account', login: 'ghost' };
      expect(hook().isUnknownLocation(known, ghost)).toBe(true);
      expect(hook().isUnknownLocation(known, ghost)).toBe(false);
    });
  });
});
