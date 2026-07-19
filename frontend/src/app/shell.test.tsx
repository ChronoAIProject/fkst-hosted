import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { MemoryRouter, Routes, Route } from 'react-router-dom';
import { Shell, nextCondensed } from './shell';
import { AuthProvider } from '@/lib/auth/github-auth';

function renderShell() {
  return render(
    <AuthProvider>
      <MemoryRouter initialEntries={['/']}>
        <Routes>
          <Route element={<Shell />}>
            <Route index element={<div>home content</div>} />
          </Route>
        </Routes>
      </MemoryRouter>
    </AuthProvider>
  );
}

describe('Shell', () => {
  it('exposes the two primary nav tabs and renders the outlet', () => {
    renderShell();
    expect(screen.getByRole('link', { name: 'Introduction' })).toBeInTheDocument();
    // "Get Started" appears as the nav tab, the header CTA, and the footer link.
    expect(screen.getAllByRole('link', { name: /get started/i }).length).toBeGreaterThanOrEqual(2);
    expect(screen.getByText('home content')).toBeInTheDocument();
  });

  it('has no backend/app tabs left over', () => {
    renderShell();
    expect(screen.queryByRole('link', { name: 'Goals' })).not.toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'Packages' })).not.toBeInTheDocument();
  });

  it('houses both the routed outlet and the footer inside the single <main> scroll region', () => {
    renderShell();
    const main = screen.getByRole('main');
    // The one scroll container must own the whole document body: content first,
    // footer last — so scrolling reveals everything and the footer keeps its
    // end-of-document semantics.
    expect(main).toContainElement(screen.getByText('home content'));
    // A <footer> nested in <main> is role-generic (not contentinfo), so query
    // the element directly and assert it is the last child of the scroll region.
    const footer = main.querySelector('footer');
    expect(footer).not.toBeNull();
    expect(main.lastElementChild).toBe(footer);
  });

  it('drives the condensed topbar off the <main> scrollTop, not the window', () => {
    // If the listener were still on window (scrollY, always 0 in this shell),
    // no amount of content scrolling would ever condense the header.
    const windowSpy = vi.spyOn(window, 'addEventListener');
    renderShell();
    expect(windowSpy).not.toHaveBeenCalledWith('scroll', expect.anything(), expect.anything());
    windowSpy.mockRestore();

    const main = screen.getByRole('main');
    const header = screen.getByRole('banner');
    expect(header.className).toContain('h-[62px]');

    // jsdom has no layout, so drive scrollTop directly and fire the event the
    // handler listens for on the main element.
    Object.defineProperty(main, 'scrollTop', { value: 200, configurable: true });
    fireEvent.scroll(main);
    expect(header.className).toContain('h-[48px]');

    Object.defineProperty(main, 'scrollTop', { value: 0, configurable: true });
    fireEvent.scroll(main);
    expect(header.className).toContain('h-[62px]');
  });
});

describe('nextCondensed', () => {
  it('condenses past 140px and expands below 40px, holding within the band', () => {
    expect(nextCondensed(false, 200)).toBe(true);
    expect(nextCondensed(true, 10)).toBe(false);
    expect(nextCondensed(true, 100)).toBe(true); // hysteresis holds previous
    expect(nextCondensed(false, 100)).toBe(false);
  });
});
