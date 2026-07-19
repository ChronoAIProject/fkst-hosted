import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, fireEvent, within } from '@testing-library/react';
import { MemoryRouter, Routes, Route } from 'react-router-dom';
import { Shell, nextCondensed } from './shell';
import { AuthProvider } from '@/lib/auth/github-auth';

const ACCESS_KEY = 'fkst-gh-access';

/** Render the shell. `authenticated` seeds the token the AuthProvider reads at
 *  init so the signed-in topbar (Environments + Sign out) is exercised. */
function renderShell({ authenticated = false }: { authenticated?: boolean } = {}) {
  if (authenticated) {
    window.localStorage.setItem(ACCESS_KEY, 'test-token');
  }
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

afterEach(() => {
  window.localStorage.clear();
  vi.unstubAllGlobals();
});

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

  it('keeps the overflow menu collapsed until the hamburger is clicked', () => {
    renderShell();
    const trigger = screen.getByRole('button', { name: 'More' });
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    // Closed: the menu body is unmounted (Reveal renders nothing when closed).
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });

  it('surfaces Sign in + GitHub + CTA in the overflow menu when signed out', () => {
    renderShell();
    fireEvent.click(screen.getByRole('button', { name: 'More' }));
    const menu = screen.getByRole('menu');
    // Signed-out: the menu carries a Sign in entry (the topbar previously had
    // none at any width) and never a Sign out.
    expect(within(menu).getByRole('menuitem', { name: /sign in with github/i })).toBeInTheDocument();
    expect(within(menu).queryByRole('menuitem', { name: /sign out/i })).not.toBeInTheDocument();
    expect(within(menu).getByRole('menuitem', { name: 'GitHub ↗' })).toBeInTheDocument();
    expect(within(menu).getByRole('menuitem', { name: /get started/i })).toBeInTheDocument();
  });

  it('closes the overflow menu on Escape', () => {
    renderShell();
    fireEvent.click(screen.getByRole('button', { name: 'More' }));
    expect(screen.getByRole('menu')).toBeInTheDocument();
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });

  it('shows Sign out (not Sign in) in the overflow menu when signed in', () => {
    renderShell({ authenticated: true });
    fireEvent.click(screen.getByRole('button', { name: 'More' }));
    const menu = screen.getByRole('menu');
    expect(within(menu).getByRole('menuitem', { name: /sign out/i })).toBeInTheDocument();
    expect(within(menu).queryByRole('menuitem', { name: /sign in/i })).not.toBeInTheDocument();
  });

  it('hides the Environments topbar entry when signed out', () => {
    renderShell();
    expect(screen.queryByRole('button', { name: 'Environments' })).not.toBeInTheDocument();
  });

  it('opens the environments drawer from the authenticated topbar entry', () => {
    // The drawer fetches profiles on open; stub the network so no real request
    // is made — the fetch failing still renders the drawer chrome.
    vi.stubGlobal('fetch', vi.fn(() => Promise.reject(new Error('no network'))));

    renderShell({ authenticated: true });
    const envButton = screen.getByRole('button', { name: 'Environments' });
    // Closed until clicked: the drawer's titled heading is absent (only the
    // topbar button carries the label).
    expect(screen.queryByRole('heading', { name: 'Environments' })).not.toBeInTheDocument();
    fireEvent.click(envButton);
    // Open: the drawer renders its heading (distinct from the topbar button).
    expect(screen.getByRole('heading', { name: 'Environments' })).toBeInTheDocument();
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
