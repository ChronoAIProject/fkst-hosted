import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, fireEvent, within } from '@testing-library/react';
import { MemoryRouter, Routes, Route } from 'react-router-dom';
import { Shell, nextCondensed } from './shell';
import { AuthProvider } from '@/lib/auth/github-auth';
import { BroaderOAuthProvider } from '@/lib/auth/broader-oauth';
import { ToastProvider } from '@/components/ui/toast';

const ACCESS_KEY = 'fkst-gh-access';

/** Render the shell. `authenticated` seeds the token the AuthProvider reads at
 *  init so the signed-in topbar (Environments + Sign out) is exercised. */
function renderShell({
  authenticated = false,
  initialEntry = '/',
}: { authenticated?: boolean; initialEntry?: string } = {}) {
  if (authenticated) {
    window.localStorage.setItem(ACCESS_KEY, 'test-token');
  }
  return render(
    <AuthProvider>
      {/* The shell now hosts the FKST Orchestrator, which forwards the
          broader-visibility credential and raises toasts — so it needs both
          providers, exactly as production does (app/index.tsx mounts them above
          the router). */}
      <BroaderOAuthProvider>
        <ToastProvider>
          <MemoryRouter initialEntries={[initialEntry]}>
            <Routes>
              <Route element={<Shell />}>
                <Route index element={<div>home content</div>} />
                <Route path="get-started" element={<div>doc content</div>} />
                <Route path="operations" element={<div>operations content</div>} />
              </Route>
            </Routes>
          </MemoryRouter>
        </ToastProvider>
      </BroaderOAuthProvider>
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
    expect(screen.getByRole('link', { name: 'Home' })).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Dashboard' })).toBeInTheDocument();
    // v2 chrome: no Get Started nav tab, header CTA, or footer link — the
    // landing hero owns the get-started entry point.
    expect(screen.queryByRole('link', { name: /get started/i })).not.toBeInTheDocument();
    expect(screen.getByText('home content')).toBeInTheDocument();
  });

  it('has no backend/app tabs left over', () => {
    renderShell();
    expect(screen.queryByRole('link', { name: 'Goals' })).not.toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'Packages' })).not.toBeInTheDocument();
  });

  it('houses the outlet and the footer inside the single <main> on doc routes', () => {
    renderShell({ initialEntry: '/get-started' });
    const main = screen.getByRole('main');
    // The one scroll container owns the whole document body on scrolling doc
    // routes: content first, footer last — so scrolling reveals everything and
    // the footer keeps its end-of-document semantics.
    expect(main).toContainElement(screen.getByText('doc content'));
    // A <footer> nested in <main> is role-generic (not contentinfo), so query
    // the element directly and assert it is the last child of the scroll region.
    const footer = main.querySelector('footer');
    expect(footer).not.toBeNull();
    expect(main.lastElementChild).toBe(footer);
    // …and the pinned bar must NOT also render (no double footer).
    expect(main.nextElementSibling?.tagName).not.toBe('FOOTER');
  });

  it('pins the footer after <main> on the full-height landing route', () => {
    renderShell();
    const main = screen.getByRole('main');
    expect(main).toContainElement(screen.getByText('home content'));
    // The landing is a single-viewport page: the footer sits OUTSIDE the
    // scroll region, pinned as main's next sibling at the viewport bottom.
    expect(main.querySelector('footer')).toBeNull();
    expect(main.nextElementSibling?.tagName).toBe('FOOTER');
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

  it('surfaces Sign in + GitHub in the overflow menu when signed out', () => {
    renderShell();
    fireEvent.click(screen.getByRole('button', { name: 'More' }));
    const menu = screen.getByRole('menu');
    // Signed-out: the menu carries a Sign in entry and never a Sign out. The
    // v2 chrome has no Get Started CTA, so the menu carries none either.
    expect(
      within(menu).getByRole('menuitem', { name: /sign in with github/i })
    ).toBeInTheDocument();
    expect(within(menu).queryByRole('menuitem', { name: /sign out/i })).not.toBeInTheDocument();
    expect(within(menu).getByRole('menuitem', { name: 'GitHub ↗' })).toBeInTheDocument();
    expect(within(menu).queryByRole('menuitem', { name: /get started/i })).not.toBeInTheDocument();
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

  it('keeps narrow-screen environment and language actions in the overflow popover', () => {
    renderShell({ authenticated: true });
    const inlineEnvironment = screen.getByRole('button', { name: 'Environments' });
    expect(inlineEnvironment.className).toContain('max-[600px]:hidden');
    expect(screen.getByRole('group', { name: 'Language' }).className).toContain(
      'max-[600px]:hidden'
    );

    fireEvent.click(screen.getByRole('button', { name: 'More' }));
    const menu = screen.getByRole('menu');
    expect(within(menu).getByRole('menuitem', { name: 'Environments' })).toBeInTheDocument();
    const popover = menu.parentElement;
    expect(popover).not.toBeNull();
    expect(within(popover!).getByRole('group', { name: 'Language' })).toBeInTheDocument();
  });

  it('hides the Environments topbar entry when signed out', () => {
    renderShell();
    expect(screen.queryByRole('button', { name: 'Environments' })).not.toBeInTheDocument();
  });

  it('opens the environments drawer from the authenticated topbar entry', () => {
    // The drawer fetches profiles on open; stub the network so no real request
    // is made — the fetch failing still renders the drawer chrome.
    vi.stubGlobal(
      'fetch',
      vi.fn(() => Promise.reject(new Error('no network')))
    );

    renderShell({ authenticated: true });
    const envButton = screen.getByRole('button', { name: 'Environments' });
    // Closed until clicked: the drawer's titled heading is absent (only the
    // topbar button carries the label).
    expect(screen.queryByRole('heading', { name: 'Environments' })).not.toBeInTheDocument();
    fireEvent.click(envButton);
    // Open: the drawer renders its heading (distinct from the topbar button).
    expect(screen.getByRole('heading', { name: 'Environments' })).toBeInTheDocument();
  });

  it('offers Operations to EVERY authenticated user, not only administrators', () => {
    renderShell({ authenticated: true });
    // The link is drawn from the locally-known session flag alone. Nothing here
    // consults an overview, an admin claim, or any other API state — the route's
    // own API is the boundary, and a regular user is entitled to the route.
    const nav = screen.getByRole('navigation');
    expect(within(nav).getByRole('link', { name: 'Operations' })).toHaveAttribute(
      'href',
      '/operations'
    );
  });

  it('styles Operations with the same nav classes as Home and Dashboard', () => {
    // Regression guard for the template-literal bug: `${navLinkClass}` inside a
    // template string stringifies the FUNCTION SOURCE, so the link ends up with
    // its source text as a className and none of the real nav styling. Asserting
    // against the classes Home actually carries makes that failure mode fail here.
    renderShell({ authenticated: true });
    const nav = screen.getByRole('navigation');
    const home = within(nav).getByRole('link', { name: 'Home' });
    const operations = within(nav).getByRole('link', { name: 'Operations' });

    const shared = ['hover-underline', 'text-nav', 'no-underline', 'rounded-control'];
    for (const cls of shared) {
      expect(home.className).toContain(cls);
      expect(operations.className).toContain(cls);
    }
    // The inactive route styling must be the shared one, not a stringified fn.
    expect(operations.className).toContain('text-faint');
    expect(operations.className).not.toContain('isActive');
    expect(operations.className).not.toContain('=>');
    // …while the responsive collapse rule is still composed on top.
    expect(operations.className).toContain('max-[720px]:hidden');
  });

  it('applies the active nav styling to Operations on /operations', () => {
    // isActive is only evaluated when React Router can CALL the className fn.
    renderShell({ authenticated: true, initialEntry: '/operations' });
    const nav = screen.getByRole('navigation');
    const operations = within(nav).getByRole('link', { name: 'Operations' });
    expect(operations.className).toContain('text-fg');
    expect(operations.className).toContain('bg-raise');
    expect(operations.className).not.toContain('text-faint');
  });

  it('keeps Operations reachable from the overflow menu at narrow widths', () => {
    // The inline nav link hides below 721px; without this menu entry the route
    // would simply vanish on a phone.
    renderShell({ authenticated: true });
    fireEvent.click(screen.getByRole('button', { name: 'More' }));
    expect(
      within(screen.getByRole('menu')).getByRole('menuitem', { name: 'Operations' })
    ).toHaveAttribute('href', '/operations');
  });

  it('hides Operations from a signed-out visitor', () => {
    renderShell();
    expect(screen.queryByRole('link', { name: 'Operations' })).not.toBeInTheDocument();
    expect(screen.queryByRole('menuitem', { name: 'Operations' })).not.toBeInTheDocument();
  });

  it('treats /operations as a fixed-height app route with no marketing footer', () => {
    renderShell({ authenticated: true, initialEntry: '/operations' });
    const main = screen.getByRole('main');
    expect(main).toContainElement(screen.getByText('operations content'));
    // Same contract as the dashboard: the scroll region holds no footer, and the
    // slim bar is pinned after <main> so the window itself never scrolls.
    expect(main.querySelector('footer')).toBeNull();
    expect(main.nextElementSibling?.tagName).toBe('FOOTER');
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
