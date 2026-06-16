import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Overview } from './overview';

describe('Overview Screen Unit Tests', () => {
  it('renders counts as — or unknown, never 0, in default empty state', () => {
    render(<Overview />);

    // Check that counts in stages render as "—", and vitals render as "unknown"
    const stageCounts = screen.getAllByText('—');
    expect(stageCounts.length).toBeGreaterThan(0);

    // Verify "0" is not rendered as a count
    // We search for text "0" that is a standalone count/element
    const allZeroElements = screen.queryAllByText('0');
    expect(allZeroElements.length).toBe(0);

    // Check vitals values render as "unknown"
    const unknownElements = screen.getAllByText('unknown');
    expect(unknownElements.length).toBeGreaterThan(0);
  });

  it('proves the forbidden string "Nothing needs you" is absent', () => {
    render(<Overview />);
    expect(screen.queryByText(/Nothing needs you/i)).toBeNull();
  });

  it('asserts the pipeline rail container classes (no wrapping)', () => {
    const { container } = render(<Overview />);
    
    // Find the pipeline scroll container by checking class names
    // The pipeline container has classes like: relative flex items-stretch border-t border-b border-line max-[600px]:flex-col overflow-x-auto
    const pipeContainer = container.querySelector('.overflow-x-auto');
    expect(pipeContainer).toBeInTheDocument();
    expect(pipeContainer).toHaveClass('flex');
    expect(pipeContainer).toHaveClass('items-stretch');
    expect(pipeContainer).toHaveClass('overflow-x-auto');
    expect(pipeContainer).not.toHaveClass('flex-wrap');
  });

  it('renders populated data when optional props are provided', () => {
    render(
      <Overview
        goals={[
          {
            id: '152',
            title: 'Composed conformance suite for github-autochrono',
            stage: 'Ship',
            state: 'merging',
            age: '3m',
          },
        ]}
        vitals={{
          inFlight: 1,
          merged24h: 5,
          deadEnded: 0,
        }}
        needsYou={[
          {
            lead: 'Merging',
            title: 'composed conformance for github-autochrono',
            id: '152',
            why: 'REAL write',
            actionLabel: 'Write posture',
          },
        ]}
      />
    );

    // Stage counts should update
    expect(screen.getAllByText('1').length).toBeGreaterThanOrEqual(1);
    // Vitals should render numeric values
    expect(screen.getByText('5')).toBeInTheDocument();
    // Needs you items should be rendered
    expect(screen.getByText(/composed conformance for github-autochrono/i)).toBeInTheDocument();
  });
});
