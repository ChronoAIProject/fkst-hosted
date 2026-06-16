import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/hooks/session-registry', () => ({
  useSessionRegistry: () => ({
    registerSession: vi.fn(),
    getSessionId: vi.fn(),
    clearSession: vi.fn(),
    clearAllSessions: vi.fn(),
  }),
}));

vi.mock('@/lib/hooks/useGoals', () => ({
  useTriggerGoal: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
  useUpdateGoal: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
  useDeleteGoal: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
}));

import { Goal } from './goal';
import type { GoalProps } from './goal';
import { mockLifecyclePopulated, mockTerminalBlocked } from '../../fixtures/goal';

describe('Goal Mock Fixtures and Posture Tests', () => {
  it('asserts fixtures satisfy GoalProps export type', () => {
    // Compile-time assertion using satisfies keyword
    const populatedFixture = mockLifecyclePopulated satisfies GoalProps;
    const blockedFixture = mockTerminalBlocked satisfies GoalProps;

    expect(populatedFixture).toBeDefined();
    expect(blockedFixture).toBeDefined();
  });

  it('asserts LifecyclePopulated merge-gate posture renders honest unknown', () => {
    render(
      <MemoryRouter>
        <Goal {...mockLifecyclePopulated} />
      </MemoryRouter>
    );

    // 1. Verify the merge gate header renders 'unknown' (due to posture being unknown)
    const mergeGateHeader = screen.getByText('Merge gate');
    expect(mergeGateHeader).toBeInTheDocument();
    
    // The status text in the header or posture displays "unknown"
    const unknownElements = screen.getAllByText(/unknown/i);
    expect(unknownElements.length).toBeGreaterThan(0);

    // 2. Verify the posture check itself renders honest unknown (represented as "—" and "posture unknown")
    expect(screen.getByText('posture unknown (deploy-time)')).toBeInTheDocument();
  });

  it('asserts TerminalBlocked timeline ends at blocked', () => {
    render(
      <MemoryRouter>
        <Goal {...mockTerminalBlocked} />
      </MemoryRouter>
    );

    // Verify the blocked text exists in the timeline
    const blockedElements = screen.getAllByText(/blocked/i);
    expect(blockedElements.length).toBeGreaterThan(0);

    // Verify the timeline ends at 'blocked' as the current (NOW) event
    expect(screen.getByText(/● NOW/i)).toBeInTheDocument();

    // Verify the blocked narrative description matches overview.html
    const narrativeElements = screen.getAllByText(/true-stall reconcile · engine gave up this framing after 3 rounds/i);
    expect(narrativeElements.length).toBeGreaterThan(0);
  });
});
