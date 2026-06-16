import { GoalStatus } from './goals';

export function goalStatusPresentation(status: GoalStatus): {
  label: string;
  tone: 'neutral' | 'green' | 'red' | 'gold' | 'amber';
} {
  switch (status) {
    case 'not_started':
      return { label: 'Not Started', tone: 'neutral' };
    case 'triggered':
      return { label: 'Triggered', tone: 'gold' };
    case 'running':
      return { label: 'Running', tone: 'green' };
    case 'stopped':
      return { label: 'Stopped', tone: 'amber' };
    case 'failed':
      return { label: 'Failed', tone: 'red' };
    default:
      return { label: 'Unknown', tone: 'neutral' };
  }
}
