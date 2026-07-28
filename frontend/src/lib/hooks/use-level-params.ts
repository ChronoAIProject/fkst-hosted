import { useCallback, useRef } from 'react';
import { useSearchParams } from 'react-router-dom';
import { levelToParams, paramsToLevel } from '@/components/canvas/level';
import type { CanvasLevel } from '@/components/canvas/level';
import type { OverviewResponse } from '@/lib/api/types';

/**
 * Keeps the dashboard's location in the URL: `/dashboard?owner=&repo=&session=`.
 *
 * All `useSearchParams` usage lives here so `dashboard.tsx` — already at the
 * 500-line limit — gains no logic, and so the read/write/validate rules are
 * readable in one place.
 *
 * Three rules make this safe against the dashboard's polling:
 *
 * - **The mount read happens exactly once** (a `useRef` latch). A poll that
 *   re-sets overview data must not re-parse the URL and yank the user back to
 *   where they started.
 * - **Params are written only from explicit navigation**, never from a data
 *   effect, for the same reason.
 * - **Writes use `replace`**, not push: browsing zoom levels must not fill the
 *   back stack, matching today's behaviour where Back leaves `/dashboard`.
 */
export function useLevelParams() {
  const [searchParams, setSearchParams] = useSearchParams();
  // `searchParams` is a live value, but the mount read must see only the FIRST
  // one; hold it in a ref so later renders cannot change what "initial" means.
  const initialRef = useRef<{ level: CanvasLevel; sessionKey?: string } | null>(null);
  if (initialRef.current == null) {
    initialRef.current = paramsToLevel(searchParams);
  }
  const validatedRef = useRef(false);

  /** The level (and session key) the URL asked for at mount. */
  const initial = initialRef.current;

  /** Write a level — and optionally the selected session — into the URL. */
  const navigateLevel = useCallback(
    (next: CanvasLevel, selectedKey?: string | null) => {
      setSearchParams(levelToParams(next, selectedKey), { replace: true });
    },
    [setSearchParams]
  );

  /** Drop every level parameter, returning the URL to a bare `/dashboard`. */
  const clearParams = useCallback(() => {
    setSearchParams(new URLSearchParams(), { replace: true });
  }, [setSearchParams]);

  /**
   * Once — after the overview loads — check that a URL-supplied owner/repo
   * actually exists, returning true when it does NOT so the caller can fall back
   * to the root and clear the params.
   *
   * Case-insensitive because GitHub logins and repository names are, and a user
   * pasting a differently-cased URL should still land in the right place.
   *
   * Runs once so a later refetch that transiently drops an account cannot fight
   * the user's navigation; the dashboard's own account-vanish effect owns that case.
   */
  const isUnknownLocation = useCallback(
    (overview: OverviewResponse | null, level: CanvasLevel): boolean => {
      if (overview == null || validatedRef.current) return false;
      validatedRef.current = true;
      if (level.kind === 'root') return false;

      const login = level.kind === 'account' ? level.login : level.owner;
      const account = overview.accounts.find(
        (candidate) => candidate.login.toLowerCase() === login.toLowerCase()
      );
      if (account == null) return true;
      if (level.kind === 'account') return false;
      return !account.repos.some(
        (candidate) => candidate.name.toLowerCase() === level.name.toLowerCase()
      );
    },
    []
  );

  return { initial, navigateLevel, clearParams, isUnknownLocation };
}
