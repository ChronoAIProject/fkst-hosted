import { describe, expect, it } from 'vitest';
import { render, screen, act } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Segmented, SegmentedList, SegmentedTrigger, SegmentedContent } from './segmented';

describe('Segmented Primitive', () => {
  it('switches panels via keyboard arrows', async () => {
    const user = userEvent.setup();
    render(
      <Segmented defaultValue="live">
        <SegmentedList>
          <SegmentedTrigger value="live" data-testid="tab-live">Live</SegmentedTrigger>
          <SegmentedTrigger value="1h" data-testid="tab-1h">1h</SegmentedTrigger>
          <SegmentedTrigger value="24h" data-testid="tab-24h">24h</SegmentedTrigger>
        </SegmentedList>
        <SegmentedContent value="live" data-testid="content-live">Live Content</SegmentedContent>
        <SegmentedContent value="1h" data-testid="content-1h">1h Content</SegmentedContent>
        <SegmentedContent value="24h" data-testid="content-24h">24h Content</SegmentedContent>
      </Segmented>
    );

    const tabLive = screen.getByTestId('tab-live');
    const tab1h = screen.getByTestId('tab-1h');

    // Initial state: "live" is active, other contents are not visible/active
    expect(tabLive).toHaveAttribute('data-state', 'active');
    expect(tab1h).toHaveAttribute('data-state', 'inactive');
    expect(screen.getByTestId('content-live')).toHaveAttribute('data-state', 'active');

    // Focus the active tab
    act(() => {
      tabLive.focus();
    });
    expect(tabLive).toHaveFocus();

    // Press arrow right to move to next tab (which automatically activates it by default in Radix Tabs)
    await user.keyboard('{ArrowRight}');

    // Now tab-1h is active
    expect(tab1h).toHaveAttribute('data-state', 'active');
    expect(tabLive).toHaveAttribute('data-state', 'inactive');
    expect(screen.getByTestId('content-1h')).toHaveAttribute('data-state', 'active');
    expect(screen.getByTestId('content-live')).toHaveAttribute('data-state', 'inactive');
  });
});
