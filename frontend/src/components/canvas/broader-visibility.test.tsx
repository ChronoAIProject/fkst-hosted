import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { BroaderVisibilityBanner } from './broader-visibility';

// useContent() falls back to the English catalog without a LanguageProvider
// (see i18n.test.tsx), so these render the banner bare and assert on EN copy.

describe('BroaderVisibilityBanner', () => {
  it('renders nothing when the feature is unavailable', () => {
    const { container } = render(
      <BroaderVisibilityBanner
        available={false}
        connected={false}
        onConnect={() => {}}
        onDisconnect={() => {}}
      />
    );
    expect(container).toBeEmptyDOMElement();
  });

  it('shows the connect invite only when available and not connected', () => {
    render(
      <BroaderVisibilityBanner
        available
        connected={false}
        onConnect={() => {}}
        onDisconnect={() => {}}
      />
    );
    expect(screen.getByText('See all your repositories')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Connect' })).toBeInTheDocument();
    // The connected chip is NOT shown in this state.
    expect(screen.queryByText('Showing all repositories')).not.toBeInTheDocument();
  });

  it('invokes onConnect when Connect is clicked', async () => {
    const user = userEvent.setup();
    const onConnect = vi.fn();
    render(
      <BroaderVisibilityBanner
        available
        connected={false}
        onConnect={onConnect}
        onDisconnect={() => {}}
      />
    );
    await user.click(screen.getByRole('button', { name: 'Connect' }));
    expect(onConnect).toHaveBeenCalledOnce();
  });

  it('is dismissible — the invite disappears after Dismiss', async () => {
    const user = userEvent.setup();
    render(
      <BroaderVisibilityBanner
        available
        connected={false}
        onConnect={() => {}}
        onDisconnect={() => {}}
      />
    );
    expect(screen.getByTestId('broader-connect')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Dismiss' }));
    expect(screen.queryByTestId('broader-connect')).not.toBeInTheDocument();
  });

  it('shows the connected chip with a disconnect action when connected', async () => {
    const user = userEvent.setup();
    const onDisconnect = vi.fn();
    render(
      <BroaderVisibilityBanner
        available
        connected
        onConnect={() => {}}
        onDisconnect={onDisconnect}
      />
    );
    expect(screen.getByText('Showing all repositories')).toBeInTheDocument();
    // The connect invite is NOT shown while connected.
    expect(screen.queryByRole('button', { name: 'Connect' })).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Disconnect' }));
    expect(onDisconnect).toHaveBeenCalledOnce();
  });
});
