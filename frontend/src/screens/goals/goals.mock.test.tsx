import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Goals } from './goals';
import type { GoalsGoal, GoalsRun } from './goals';
import { mockGoals, mockRuns, mockVitals } from '../../fixtures/goals';

describe('Goals Mock Fixtures Tests', () => {
  it('asserts fixtures satisfy GoalsGoal and GoalsRun export types', () => {
    // Compile-time assertion using satisfies keyword
    const goalsFixture = mockGoals satisfies GoalsGoal[];
    const runsFixture = mockRuns satisfies GoalsRun[];

    expect(goalsFixture).toBeDefined();
    expect(runsFixture).toBeDefined();
  });

  it('renders issues list correctly in Issues view', () => {
    render(<Goals view="issues" goals={mockGoals} />);

    // Verify goals are rendered in list
    expect(screen.getByText('Tighten consensus parser to handle nested quorum refs')).toBeInTheDocument();
    expect(screen.getByText('Document DLQ retention & replay semantics for the delivery ledger')).toBeInTheDocument();
  });

  it('renders runs and vitals correctly in Activity view', () => {
    render(<Goals view="activity" vitals={mockVitals} runs={mockRuns} />);

    // Verify vitals render correctly
    expect(screen.getByText('~94%')).toBeInTheDocument();

    // Verify runs render correctly
    expect(screen.getByText('Make the state label set-exclusive on PR open')).toBeInTheDocument();
    
    // Verify that merge · REAL → integration run renders
    expect(screen.getByText('Extract cache_get / cache_set helper functions')).toBeInTheDocument();
  });
});
