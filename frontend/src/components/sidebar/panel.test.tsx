import { describe, it, expect } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import type { CanvasLevel } from '@/components/canvas/level';
import { SidebarPanel } from './panel';

const root: CanvasLevel = { kind: 'root' };
const account: CanvasLevel = { kind: 'account', login: 'acme' };

describe('SidebarPanel', () => {
  it('renders the labelled shell and its body inside an internal scroll region', () => {
    render(
      <SidebarPanel level={root}>
        <p>body content</p>
      </SidebarPanel>
    );

    // Shell is the accessible landmark; its body is rendered.
    const aside = screen.getByLabelText('Details panel');
    expect(aside.tagName).toBe('ASIDE');
    expect(screen.getByText('body content')).toBeInTheDocument();

    // Scrolling is delegated to a flex-safe internal ScrollArea, never the
    // shell/page: the region carries the flex-1 min-h-0 overflow-y-auto trio.
    const region = aside.querySelector('.overflow-y-auto');
    expect(region).not.toBeNull();
    expect(region).toHaveClass('flex-1', 'min-h-0', 'overflow-y-auto');
  });

  it('drops the magic 720px cap and the narrow max-h-none escape hatch', () => {
    render(
      <SidebarPanel level={root}>
        <p>body</p>
      </SidebarPanel>
    );
    const aside = screen.getByLabelText('Details panel');

    // The former height hacks are gone…
    expect(aside.className).not.toContain('max-h-[720px]');
    expect(aside.className).not.toContain('max-h-none');

    // …replaced by a flex column that fills the row on desktop and keeps a
    // sensible min-height floor when stacked (<=1100px).
    expect(aside).toHaveClass('flex', 'flex-col', 'min-h-0', 'h-full');
    expect(aside.className).toContain('max-[1100px]:min-h-[22rem]');
    expect(aside.className).toContain('max-[1100px]:h-auto');
  });

  it('crossfades to the new body when the level changes', async () => {
    const { rerender } = render(
      <SidebarPanel level={root}>
        <p>root body</p>
      </SidebarPanel>
    );
    expect(screen.getByText('root body')).toBeInTheDocument();

    rerender(
      <SidebarPanel level={account}>
        <p>account body</p>
      </SidebarPanel>
    );

    // The keyed crossfade swaps the level body in; the old one leaves.
    await waitFor(() => expect(screen.getByText('account body')).toBeInTheDocument());
    await waitFor(() => expect(screen.queryByText('root body')).not.toBeInTheDocument());
  });

  it('crossfades skeleton→content within a level via the loaded flag', async () => {
    const { rerender } = render(
      <SidebarPanel level={account} loaded={false}>
        <p>skeleton</p>
      </SidebarPanel>
    );
    expect(screen.getByText('skeleton')).toBeInTheDocument();

    // Same level, load-state flips: the key encodes `loaded`, so this animates
    // as a swap rather than replacing content in place with no transition.
    rerender(
      <SidebarPanel level={account} loaded={true}>
        <p>loaded body</p>
      </SidebarPanel>
    );
    await waitFor(() => expect(screen.getByText('loaded body')).toBeInTheDocument());
    await waitFor(() => expect(screen.queryByText('skeleton')).not.toBeInTheDocument());
  });
});
