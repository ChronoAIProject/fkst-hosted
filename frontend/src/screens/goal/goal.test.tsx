import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { describe, expect, it } from 'vitest';
import { Goal } from './goal';

describe('Goal Screen Unit Tests', () => {
  it('renders counts as — or unknown, never 0, in default empty state', () => {
    render(
      <MemoryRouter>
        <Goal />
      </MemoryRouter>
    );

    // Check that state, version, headSha, branch etc render as "—" or "unknown"
    const unknownElements = screen.getAllByText('unknown');
    expect(unknownElements.length).toBeGreaterThan(0);

    // Verify "0" is not rendered as a count
    const allZeroElements = screen.queryAllByText('0');
    expect(allZeroElements.length).toBe(0);

    // Verify empty state timeline text is rendered
    expect(screen.getByText(/no GitHub plane connected — sign-in pending/i)).toBeInTheDocument();
    
    // Verify panels show host telemetry not connected
    const telemetryElements = screen.getAllByText(/host telemetry not connected/i);
    expect(telemetryElements.length).toBeGreaterThanOrEqual(2);
  });

  it('proves the forbidden string "Nothing needs you" is absent', () => {
    render(
      <MemoryRouter>
        <Goal />
      </MemoryRouter>
    );
    expect(screen.queryByText(/Nothing needs you/i)).toBeNull();
  });

  it('renders populated goal page correctly when props are provided', () => {
    render(
      <MemoryRouter>
        <Goal
          goalId="152"
          title="Composed conformance for github-autochrono"
          state="merge-ready"
          version="2026-06-11T00-12Z"
          headSha="3c1a9f"
          branch="fkst/cand-152"
          blocksGoalId="242"
          pr={{ number: 29, href: 'https://github.com/foo/bar/pull/29' }}
          isReal={true}
          mergeGate={{
            reviewApproved: 'ok',
            headBound: 'ok',
            ciGreen: 'ok',
            mergeable: 'ok',
            posture: 'ok',
          }}
          consensus={{
            summary: 'PR diff reviewed successfully',
            passes: true,
          }}
          lifecycleEvents={[
            {
              name: 'thinking',
              timestamp: '00:41Z · 41m ago',
              body: 'intake: fkst-dev:enabled',
            },
          ]}
          deliveries={[
            {
              status: 'ACK',
              name: 'review_result',
              gen: 5,
              state: 'done',
            },
          ]}
          runs={[
            {
              exitCode: 0,
              action: 'meta-judge',
              duration: '17s',
            },
          ]}
        />
      </MemoryRouter>
    );

    // Title and IDs should be rendered
    expect(screen.getByText('Composed conformance for github-autochrono')).toBeInTheDocument();
    expect(screen.getByText(/#152/i)).toBeInTheDocument();
    expect(screen.getByText(/PR #29/i)).toBeInTheDocument();
    expect(screen.getByText('3c1a9f')).toBeInTheDocument();
    expect(screen.getByText('#242')).toBeInTheDocument();

    // Lifecycle timeline item
    expect(screen.getByText('thinking')).toBeInTheDocument();
    expect(screen.getByText('00:41Z · 41m ago')).toBeInTheDocument();

    // Deliveries and runs
    expect(screen.getByText(/review_result · gen 5 · done/i)).toBeInTheDocument();
    expect(screen.getByText(/exit 0/i)).toBeInTheDocument();

    // Merge gate verdict and consensus check
    expect(screen.getByText('passing')).toBeInTheDocument();
    expect(screen.getByText('consensus passed')).toBeInTheDocument();
    expect(screen.getByText('PR diff reviewed successfully')).toBeInTheDocument();
  });
});
