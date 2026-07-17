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
