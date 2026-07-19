import { describe, it, expect } from 'vitest';
import { createRef } from 'react';
import { render } from '@testing-library/react';
import { ScrollArea } from './scroll-area';

/** The scrolling element is the component's single rendered node. */
function scrollEl(container: HTMLElement): HTMLDivElement {
  return container.firstElementChild as HTMLDivElement;
}

describe('ScrollArea', () => {
  it('applies the flex-safe pattern including the mandatory min-h-0', () => {
    const { container } = render(<ScrollArea>body</ScrollArea>);
    const el = scrollEl(container);
    // min-h-0 is the load-bearing class: without it the flex child never
    // shrinks and the page scrolls instead of this region.
    expect(el).toHaveClass('flex-1', 'min-h-0', 'overflow-y-auto');
    expect(el.textContent).toBe('body');
  });

  it('forwards the ref to the scrolling element so callers can read scrollTop', () => {
    const ref = createRef<HTMLDivElement>();
    const { container } = render(<ScrollArea ref={ref}>body</ScrollArea>);
    expect(ref.current).toBe(scrollEl(container));
    // scrollTop is readable on the forwarded node (0 in jsdom).
    expect(ref.current?.scrollTop).toBe(0);
  });

  it("defaults to the y axis: vertical scroll, horizontal hidden", () => {
    const { container } = render(<ScrollArea>body</ScrollArea>);
    const el = scrollEl(container);
    expect(el).toHaveClass('overflow-y-auto', 'overflow-x-hidden');
    expect(el).not.toHaveClass('overflow-x-auto');
  });

  it("axis='both' enables horizontal scrolling too", () => {
    const { container } = render(<ScrollArea axis="both">body</ScrollArea>);
    const el = scrollEl(container);
    expect(el).toHaveClass('overflow-y-auto', 'overflow-x-auto');
    expect(el).not.toHaveClass('overflow-x-hidden');
  });

  it('normalizes a numeric maxHeight to px and passes strings through', () => {
    const { container: c1 } = render(<ScrollArea maxHeight={720}>body</ScrollArea>);
    expect(scrollEl(c1).style.maxHeight).toBe('720px');

    const { container: c2 } = render(<ScrollArea maxHeight="60vh">body</ScrollArea>);
    expect(scrollEl(c2).style.maxHeight).toBe('60vh');
  });

  it('leaves maxHeight unset when the prop is omitted', () => {
    const { container } = render(<ScrollArea>body</ScrollArea>);
    expect(scrollEl(container).style.maxHeight).toBe('');
  });

  it('applies the thin token-tinted scrollbar styling', () => {
    const { container } = render(<ScrollArea>body</ScrollArea>);
    const el = scrollEl(container);
    expect(el.style.scrollbarWidth).toBe('thin');
    expect(el.style.scrollbarColor).toBe('var(--line-2) transparent');
  });

  it('merges a passthrough className without dropping the base classes', () => {
    const { container } = render(<ScrollArea className="p-5">body</ScrollArea>);
    const el = scrollEl(container);
    expect(el).toHaveClass('p-5', 'min-h-0', 'flex-1');
  });
});
