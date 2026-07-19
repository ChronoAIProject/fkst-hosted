import type { SessionPhase, WorkItemTone } from '@/lib/api/derive';

export type ChipTone = 'neutral' | 'amber' | 'green' | 'red';

/** Chip tone for a lifecycle phase. The live phases stay amber (brand-adjacent),
 *  terminal-bad phases go red, retired/idle read neutral, active reads green. */
export const PHASE_TONE: Record<SessionPhase, ChipTone> = {
  registered: 'amber',
  active: 'green',
  'picked-up': 'amber',
  degraded: 'red',
  retired: 'neutral',
  invalid: 'red',
  idle: 'neutral',
};

/** Map a decoded work-item tone to a Chip tone. */
export const WORK_TONE: Record<WorkItemTone, ChipTone> = {
  neutral: 'neutral',
  progress: 'amber',
  good: 'green',
  bad: 'red',
};
