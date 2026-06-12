import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Goals } from './goals';

describe('Goals Screen Unit Tests', () => {
  it('renders counts as — or unknown, never 0, in default empty state', () => {
    render(<Goals />);

    // Check that goal counts and age render as "—" or "unknown"
    const dashes = screen.getAllByText('—');
    expect(dashes.length).toBeGreaterThan(0);

    // Verify "0" is not rendered as a count
    const allZeroElements = screen.queryAllByText('0');
    expect(allZeroElements.length).toBe(0);

    // Verify empty state text is rendered
    expect(screen.getByText(/no GitHub plane connected — sign-in pending/i)).toBeInTheDocument();
  });

  it('proves the forbidden string "Nothing needs you" is absent', () => {
    render(<Goals />);
    expect(screen.queryByText(/Nothing needs you/i)).toBeNull();
  });

  it('renders the Activity view empty state when view="activity" is passed', () => {
    render(<Goals view="activity" />);

    // Vitals and run lists should read "—" or "unknown"
    const dashes = screen.getAllByText('—');
    expect(dashes.length).toBeGreaterThan(0);

    // Verify Activity empty state note is present
    expect(screen.getByText(/host telemetry not connected/i)).toBeInTheDocument();
  });

  it('renders custom populated data in both views', () => {
    // 1. Populated Issues
    const { rerender } = render(
      <Goals
        view="issues"
        goals={[
          {
            id: '152',
            title: 'Composed conformance suite',
            stage: 'Ship',
            state: 'merging',
            age: '3m',
            repo: 'fkst-substrate',
            pr: '#29',
            ci: 'passing',
          },
        ]}
      />
    );

    expect(screen.getByText('Composed conformance suite')).toBeInTheDocument();
    expect(screen.getByText('152')).toBeInTheDocument();

    // 2. Populated Activity
    rerender(
      <Goals
        view="activity"
        vitals={{
          runsDispatched: '10',
          successRate: '90%',
          medianDuration: '30s',
          inDlq: 'unknown',
        }}
        runs={[
          {
            id: 'run_1',
            goalId: '205',
            goalTitle: 'State label set-exclusive',
            action: 'implement',
            attempt: '1',
            duration: '38s',
            exitCode: 0,
            when: 'just now',
            lease: '1',
            status: 'ok',
          },
        ]}
      />
    );

    expect(screen.getByText('10')).toBeInTheDocument();
    expect(screen.getByText('90%')).toBeInTheDocument();
    expect(screen.getByText('State label set-exclusive')).toBeInTheDocument();
  });
});
