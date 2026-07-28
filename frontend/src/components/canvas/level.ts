// The canvas's three zoom levels. A plain discriminated union so the page,
// the flow, and the sidebar all agree on what is being looked at.

export type CanvasLevel =
  | { kind: 'root' }
  | { kind: 'account'; login: string }
  | { kind: 'repo'; owner: string; name: string };

/** Stable string identity for effects/keys reacting to level changes. */
export function levelKey(level: CanvasLevel): string {
  switch (level.kind) {
    case 'root':
      return 'root';
    case 'account':
      return `account:${level.login}`;
    case 'repo':
      return `repo:${level.owner}/${level.name}`;
  }
}

/** One step up (repo → its account → root); null at the root. */
export function parentLevel(level: CanvasLevel): CanvasLevel | null {
  switch (level.kind) {
    case 'root':
      return null;
    case 'account':
      return { kind: 'root' };
    case 'repo':
      return { kind: 'account', login: level.owner };
  }
}

/** Query-parameter names carrying a dashboard location. Kept here beside the
 *  level type so the mapping has exactly one home. */
export const LEVEL_PARAM_OWNER = 'owner';
export const LEVEL_PARAM_REPO = 'repo';
export const LEVEL_PARAM_SESSION = 'session';

/** A level (plus an optional selected session) as URL query parameters.
 *
 *  Pure string mapping — no validation against loaded data, which is the caller's
 *  job once the overview arrives. The root level maps to no parameters at all, so
 *  `/dashboard` stays the canonical clean URL. */
export function levelToParams(level: CanvasLevel, selectedKey?: string | null): URLSearchParams {
  const params = new URLSearchParams();
  if (level.kind === 'account') {
    params.set(LEVEL_PARAM_OWNER, level.login);
  } else if (level.kind === 'repo') {
    params.set(LEVEL_PARAM_OWNER, level.owner);
    params.set(LEVEL_PARAM_REPO, level.name);
    // A session only means something inside a repo, so it is never written alone.
    if (selectedKey) params.set(LEVEL_PARAM_SESSION, selectedKey);
  }
  return params;
}

/** URL query parameters back to a level (plus any session key).
 *
 *  Deliberately TOLERANT: a hand-typed or truncated URL must degrade to the nearest
 *  sensible level rather than render something broken. `repo` without `owner` is
 *  ignored (there is no repo without an account), and `session` without `repo` is
 *  ignored (nothing to select in). */
export function paramsToLevel(params: URLSearchParams): {
  level: CanvasLevel;
  sessionKey?: string;
} {
  const owner = params.get(LEVEL_PARAM_OWNER)?.trim();
  const repo = params.get(LEVEL_PARAM_REPO)?.trim();
  const session = params.get(LEVEL_PARAM_SESSION)?.trim();

  if (!owner) return { level: { kind: 'root' } };
  if (!repo) return { level: { kind: 'account', login: owner } };
  return {
    level: { kind: 'repo', owner, name: repo },
    ...(session ? { sessionKey: session } : {}),
  };
}
