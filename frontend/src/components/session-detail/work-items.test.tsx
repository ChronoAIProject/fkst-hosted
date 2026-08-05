import { describe, it, expect } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import type { IssueDetail } from '@/lib/api/types';
import { WorkItemsPane } from './work-items';

const issue = (over: Partial<IssueDetail> & Pick<IssueDetail, 'number'>): IssueDetail => ({
  title: `issue ${over.number}`,
  state: 'open',
  author: 'shining',
  labels: [],
  html_url: `https://github.com/o/r/issues/${over.number}`,
  created_at: '2026-07-01T00:00:00Z',
  updated_at: '2026-07-01T00:00:00Z',
  closed_at: null,
  ...over,
});

describe('WorkItemsPane', () => {
  it('renders one row per item, linking both the number and the title', () => {
    render(
      <WorkItemsPane
        issues={[issue({ number: 9, title: 'do the thing', labels: ['fkst-dev:implementing'] })]}
      />
    );
    const pane = screen.getByRole('region', { name: 'Work items' });
    expect(within(pane).getByRole('link', { name: '#9' })).toHaveAttribute(
      'href',
      'https://github.com/o/r/issues/9'
    );
    expect(within(pane).getByRole('link', { name: 'do the thing' })).toHaveAttribute(
      'href',
      'https://github.com/o/r/issues/9'
    );
    // The decoded state chip travels with the row.
    expect(within(pane).getByText('Implementing')).toBeInTheDocument();
  });

  it('counts the items beside the label, and omits the count when empty', () => {
    const { rerender } = render(
      <WorkItemsPane issues={[issue({ number: 9 }), issue({ number: 10 })]} />
    );
    expect(screen.getByText('· 2')).toBeInTheDocument();

    rerender(<WorkItemsPane issues={[]} />);
    expect(screen.queryByText(/· \d/)).not.toBeInTheDocument();
  });

  it('shows the empty note instead of an empty grid', () => {
    render(<WorkItemsPane issues={[]} />);
    const pane = screen.getByRole('region', { name: 'Work items' });
    expect(within(pane).getByText('No work items yet.')).toBeInTheDocument();
    // No scroll container is rendered for nothing.
    expect(pane.querySelector('.overflow-y-auto')).toBeNull();
  });

  it('scrolls a long backlog inside the pane', () => {
    // Beside the timeline (#5842), the list must overflow INSIDE its pane rather
    // than growing the grid row and scrolling the timeline out of view.
    render(
      <WorkItemsPane issues={Array.from({ length: 20 }, (_, i) => issue({ number: i + 1 }))} />
    );
    const pane = screen.getByRole('region', { name: 'Work items' });
    expect(pane.querySelector('.overflow-y-auto')).not.toBeNull();
    expect(within(pane).getAllByRole('link', { name: /^#\d+$/ })).toHaveLength(20);
  });

  it('takes its grid-item sizing from the caller', () => {
    // The class lands on the card itself: an extra wrapper would become the grid
    // item, and the min-h-0 the caller needs would no longer be seen by the
    // track-sizing algorithm.
    render(<WorkItemsPane issues={[]} className="min-h-0" />);
    expect(screen.getByRole('region', { name: 'Work items' })).toHaveClass('min-h-0');
  });
});
