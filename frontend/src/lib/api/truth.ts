import { SessionStatus } from './types';

export type GoalState =
  | 'thinking'
  | 'ready'
  | 'implementing'
  | 'pr-open'
  | 'reviewing'
  | 'merge-ready'
  | 'merging'
  | 'fixing'
  | 'review-meta'
  | 'impl-failed'
  | 'blocked'
  | 'merged';

export type GoalStage = 'Design' | 'Build' | 'Review' | 'Ship' | 'Blocked' | 'Merged';

/**
 * Maps the 12 hyphenated goal states to their corresponding stages.
 * Contract:
 * - thinking, ready -> Design
 * - implementing, pr-open -> Build
 * - reviewing, fixing, review-meta -> Review
 * - merge-ready, merging -> Ship
 * - impl-failed, blocked -> Blocked
 * - merged -> Merged
 */
export const STAGE_BY_STATE: Record<GoalState, GoalStage> = {
  thinking: 'Design',
  ready: 'Design',
  implementing: 'Build',
  'pr-open': 'Build',
  reviewing: 'Review',
  fixing: 'Review',
  'review-meta': 'Review',
  'merge-ready': 'Ship',
  merging: 'Ship',
  'impl-failed': 'Blocked',
  blocked: 'Blocked',
  merged: 'Merged',
};

/**
 * Returns whether a given goal state is terminal (i.e. has no out-edges).
 * Terminal states: 'impl-failed', 'blocked', 'merged'.
 */
export function isTerminal(state: GoalState): boolean {
  return state === 'impl-failed' || state === 'blocked' || state === 'merged';
}

/**
 * Returns whether a session status is terminal ('stopped' or 'failed').
 */
export function isSessionTerminal(status: SessionStatus): boolean {
  return status === 'stopped' || status === 'failed';
}

/**
 * Returns the count or 'unknown' depending on source reachability.
 * If sourceReachable is false or value is undefined, it returns 'unknown'.
 * Otherwise, it returns the number value.
 */
export function countOrUnknown(
  value: number | undefined,
  sourceReachable: boolean
): number | 'unknown' {
  if (!sourceReachable || value === undefined) {
    return 'unknown';
  }
  return value;
}
