import { useEffect, useRef, useState } from 'react';
import { Link, NavLink, useLocation, useOutlet } from 'react-router-dom';
import { FkstMark } from '../components/brand/fkst-mark';
import { LanguageToggle } from '../components/layout/language-toggle';
import { RouteTransition } from '../components/ui/motion';
import { EnvironmentsDrawer } from '@/components/environments/environments-drawer';
import { TourOverlay } from '@/components/tour/tour-overlay';
import { useTour } from '@/components/tour/tour-context';
import { useContent } from '../i18n';
import { REPO, MANUAL_URL } from '../i18n/literals';
import { useAuth } from '../lib/auth/github-auth';

export function nextCondensed(prev: boolean, y: number): boolean {
  if (y > 140) {
    return true;
  }
  if (y < 40) {
    return false;
  }
  return prev;
}

// Nav links carry the underline-grow hover cue (`.hover-underline` scales a
// gradient underline in from the left); the active route keeps its raised pill
// so the current page reads without relying on the hover-only underline.
const navLinkClass = ({ isActive }: { isActive: boolean }) =>
  `hover-underline text-nav no-underline px-3 py-[7px] rounded-control transition-colors ${
    isActive
      ? 'text-fg bg-raise'
      : 'text-faint hover:text-dim'
  }`;

/** Shared mono action styling for the inline auth/GitHub/CTA topbar controls. */
const inlineActionClass =
  'font-mono text-[12px] text-faint hover:text-fg no-underline px-2.5 py-[7px] rounded-control transition-colors cursor-pointer';

/** Full-width row styling for the same actions inside the overflow menu. */
const menuItemClass =
  'font-mono text-[12px] text-left text-faint hover:text-fg no-underline px-2.5 py-2 rounded-control transition-colors cursor-pointer';

export function Shell() {
  const [condensed, setCondensed] = useState(false);
  // Local UI state for the two shell-owned surfaces: the responsive overflow
  // menu and the environments manager drawer.
  const [menuOpen, setMenuOpen] = useState(false);
  const [envOpen, setEnvOpen] = useState(false);
  const c = useContent();
  const { isAuthenticated, signIn, signOut } = useAuth();
  // The guided tour is launched on demand from the topbar `?` control.
  const { start: startTour } = useTour();
  const location = useLocation();
  // useOutlet() snapshots the CURRENT route element; passing that (not a live
  // <Outlet/>) into the keyed crossfade lets the exiting frame keep the old
  // route while the entering frame shows the new one — a live <Outlet/> would
  // render the destination in BOTH frames (double-mount) during the transition.
  const outlet = useOutlet();
  // The dashboard is a fixed-viewport app view: it fills <main> exactly and its
  // panels scroll internally, so it must NOT sit inside the auto-height padded
  // wrapper (that collapses its h-full chain) and carries no marketing footer.
  // Doc/marketing routes keep the padded, footer-terminated scrolling layout.
  const isApp = location.pathname.startsWith('/dashboard');
  const menuRef = useRef<HTMLDivElement>(null);
  // The single <main> is the app's sole scroll region (the body's overflow is
  // clipped by the css foundation), so the condense heuristic must observe THAT
  // element's scrollTop — window.scrollY stays 0 in a fixed-viewport shell.
  const mainRef = useRef<HTMLElement>(null);

  useEffect(() => {
    const el = mainRef.current;
    // Guard the null ref: if the region never mounted there is nothing to
    // observe, and reading scrollTop off null would throw on mount.
    if (!el) {
      return;
    }
    const handleScroll = () => {
      setCondensed((prev) => nextCondensed(prev, el.scrollTop));
    };
    el.addEventListener('scroll', handleScroll, { passive: true });
    handleScroll(); // initial sync (e.g. restored scroll position)
    return () => el.removeEventListener('scroll', handleScroll);
  }, []);

  // Dismiss the overflow menu on an outside click or Escape. Listeners are only
  // bound while the menu is open so the closed shell adds no global handlers.
  useEffect(() => {
    if (!menuOpen) {
      return;
    }
    const onPointerDown = (e: PointerEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setMenuOpen(false);
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setMenuOpen(false);
      }
    };
    document.addEventListener('pointerdown', onPointerDown);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('pointerdown', onPointerDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [menuOpen]);

  // A route change while the menu is open should not leave it hanging over the
  // new page.
  useEffect(() => {
    setMenuOpen(false);
  }, [location.pathname]);

  return (
    <div className="h-[100dvh] bg-bg bg-bg-glow bg-fixed text-fg font-ui flex flex-col overflow-hidden">
      <div className="max-w-shell w-full mx-auto px-6 max-[480px]:px-4 flex-1 min-h-0 flex flex-col">
        {/* pinned topbar (the column itself never scrolls, so no sticky needed).
            Refined glass bar: translucent --raise fill + backdrop-blur floats it
            over the app bloom, a soft shadow-1 separates it, and a gradient
            hairline (below) replaces the flat bottom border for a lit edge. */}
        <div className="flex-none z-40 relative bg-glass backdrop-blur-glass shadow-1">
          <header
            className={`flex items-center gap-4 transition-[height] duration-200 motion-reduce:transition-none ${
              condensed ? 'h-[48px]' : 'h-[62px]'
            }`}
          >
            <Link
              to="/"
              className="text-fg no-underline inline-block flex-none"
              aria-label={c.nav.homeAria}
            >
              {/* Wordmark clips the amber→gold brand gradient into the letters
                  (`.grad-text`); the mark's own amber counter-dot renders on top
                  unaffected (it is a separately-filled child element). */}
              <FkstMark className="text-[19px] grad-text" />
            </Link>

            <nav className="flex gap-0.5">
              <NavLink to="/" end className={navLinkClass}>
                {c.nav.introduction}
              </NavLink>
              <NavLink to="/get-started" className={navLinkClass}>
                {c.nav.getStarted}
              </NavLink>
              <NavLink to="/dashboard" className={navLinkClass}>
                {c.nav.dashboard}
              </NavLink>
            </nav>

            <div className="flex items-center gap-2 ml-auto">
              <LanguageToggle />

              {/* Environments manager entry — authenticated users only. Kept
                  visible at every width (short label); toggles the drawer. */}
              {isAuthenticated && (
                <button
                  type="button"
                  onClick={() => setEnvOpen(true)}
                  data-tour="environments"
                  className={`${inlineActionClass} flex-none`}
                >
                  {c.nav.environments}
                </button>
              )}

              {/* Guided-tour launcher — re-opens the tour on demand, ignoring
                  the per-login seen flag. Named by aria-label (glyph is
                  decorative). */}
              <button
                type="button"
                onClick={startTour}
                data-tour="help"
                aria-label={c.tour.helpAria}
                className="inline-flex items-center justify-center w-8 h-8 flex-none rounded-control text-faint hover:text-fg hover:bg-raise transition-colors cursor-pointer"
              >
                <span aria-hidden="true" className="text-[14px] leading-none font-semibold">
                  ?
                </span>
              </button>

              {/* Inline auth action. Sign in was previously ABSENT from the
                  topbar entirely; it now sits here (signed-out) and Sign out
                  here (signed-in). Both progressively hide below 600px but stay
                  reachable through the overflow menu. */}
              {isAuthenticated ? (
                <button
                  type="button"
                  onClick={signOut}
                  className={`${inlineActionClass} max-[600px]:hidden`}
                >
                  {c.auth.signOut}
                </button>
              ) : (
                <button
                  type="button"
                  onClick={signIn}
                  className={`${inlineActionClass} max-[600px]:hidden`}
                >
                  {c.auth.signIn}
                </button>
              )}

              <a
                href={REPO}
                target="_blank"
                rel="noreferrer"
                className={`${inlineActionClass} max-[720px]:hidden`}
              >
                GitHub ↗
              </a>
              <NavLink
                to="/get-started"
                // Primary CTA: brand gradient fill + card depth & amber bloom
                // (`shadow-glow`); hover lifts brightness and glow together.
                className="font-ui font-semibold text-[12.5px] bg-grad-accent text-amber-ink shadow-glow rounded-control px-3.5 py-[7px] flex-none no-underline transition-[filter,box-shadow] duration-150 hover:brightness-110 max-[480px]:hidden"
              >
                {c.nav.getStartedCta}
              </NavLink>

              {/* Responsive overflow menu — shown once the first inline item
                  (GitHub, ≤720px) starts collapsing, so the auth action, the
                  GitHub link, and the CTA are never unreachable on a narrow
                  viewport. Hidden at ≥721px where every inline item shows. */}
              <div ref={menuRef} className="relative flex-none min-[721px]:hidden">
                <button
                  type="button"
                  onClick={() => setMenuOpen((open) => !open)}
                  aria-haspopup="menu"
                  aria-expanded={menuOpen}
                  aria-label={c.nav.menuAria}
                  className="inline-flex items-center justify-center w-8 h-8 rounded-control text-faint hover:text-fg hover:bg-raise transition-colors cursor-pointer"
                >
                  {/* Decorative glyph; the control is named by aria-label. */}
                  <span aria-hidden="true" className="text-[15px] leading-none">
                    ☰
                  </span>
                </button>

                {menuOpen && (
                  <div
                    role="menu"
                    className="anim-notice-in absolute right-0 top-[calc(100%+6px)] z-50 min-w-[168px] rounded-control border border-line bg-glass backdrop-blur-glass shadow-modal-seat flex flex-col p-1"
                  >
                    {isAuthenticated ? (
                      <button
                        type="button"
                        role="menuitem"
                        onClick={() => {
                          setMenuOpen(false);
                          signOut();
                        }}
                        className={menuItemClass}
                      >
                        {c.auth.signOut}
                      </button>
                    ) : (
                      <button
                        type="button"
                        role="menuitem"
                        onClick={() => {
                          setMenuOpen(false);
                          signIn();
                        }}
                        className={menuItemClass}
                      >
                        {c.auth.signIn}
                      </button>
                    )}
                    <a
                      role="menuitem"
                      href={REPO}
                      target="_blank"
                      rel="noreferrer"
                      onClick={() => setMenuOpen(false)}
                      className={menuItemClass}
                    >
                      GitHub ↗
                    </a>
                    <NavLink
                      role="menuitem"
                      to="/get-started"
                      onClick={() => setMenuOpen(false)}
                      className={menuItemClass}
                    >
                      {c.nav.getStartedCta}
                    </NavLink>
                  </div>
                )}
              </div>
            </div>
          </header>
          {/* Gradient hairline bottom edge — a top-lit 1px sweep in place of the
              flat border, reading like light catching the bar's lower lip. Pure
              paint, no layout, ignores pointer events. */}
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-x-0 bottom-0 h-px bg-grad-hairline"
          />
        </div>

        {/* The SOLE scroll container: topbar stays pinned above; both the routed
            content and the footer scroll together inside here (the footer keeps
            its end-of-document position). */}
        <main ref={mainRef} className="flex-1 min-h-0 overflow-y-auto">
          {/* Keyed on the pathname so a route change crossfades on the shared
              curve; collapses to an instant swap under reduced motion. On the
              app route the transition is h-full (main is a definite-height flex
              item, so h-full resolves and the dashboard fills it); on doc routes
              it is auto-height so its content overflows and <main> scrolls. */}
          <RouteTransition
            k={location.pathname}
            className={isApp ? 'h-full' : ''}
          >
            <div className={isApp ? 'h-full pb-3' : 'py-10 max-[480px]:py-8'}>{outlet}</div>
          </RouteTransition>

          {/* Marketing footer — only on doc/marketing routes (it scrolls in at
              the end of content). The app dashboard omits it so it can own the
              full viewport without a scroll to reveal the footer. */}
          {!isApp && (
            <footer className="border-t border-line py-7 flex items-center gap-x-6 gap-y-2 flex-wrap font-mono text-[11.5px] text-ghost">
              <span className="flex items-center gap-2">
                <FkstMark className="text-[13px] text-dim" />
                <span>{c.footer.tagline}</span>
              </span>
              <NavLink to="/get-started" className="text-faint hover:text-fg no-underline">
                {c.footer.getStarted}
              </NavLink>
              <a href={REPO} target="_blank" rel="noreferrer" className="text-faint hover:text-fg no-underline">
                {c.footer.github}
              </a>
              <a href={MANUAL_URL} target="_blank" rel="noreferrer" className="text-faint hover:text-fg no-underline">
                {c.footer.manual}
              </a>
            </footer>
          )}
        </main>

        {/* Slim pinned footer — app route only. Sits as a flex-none row AFTER
            <main> so topbar + main(flex-1) + this footer sum to the column
            height: the dashboard keeps its internal scroll, this bar stays
            pinned at the viewport bottom, and the window never scrolls. A
            gradient hairline top edge + subtle glass mirror the topbar; it
            reuses the same c.footer.* strings + REPO/MANUAL_URL the marketing
            footer carries, in a compact ~44px bar. */}
        {isApp && (
          <footer className="flex-none relative bg-glass backdrop-blur-glass h-[44px] flex items-center gap-x-5 gap-y-1 flex-wrap px-1 font-mono text-[11px] text-ghost">
            {/* Gradient hairline top edge — a lit sweep mirroring the topbar's
                bottom lip; pure paint, no layout, ignores pointer events. */}
            <div
              aria-hidden="true"
              className="pointer-events-none absolute inset-x-0 top-0 h-px bg-grad-hairline"
            />
            <span className="flex items-center gap-2">
              <FkstMark className="text-[12px] text-dim" />
              <span>{c.footer.tagline}</span>
            </span>
            <NavLink to="/get-started" className="text-faint hover:text-fg no-underline">
              {c.footer.getStarted}
            </NavLink>
            <a href={REPO} target="_blank" rel="noreferrer" className="text-faint hover:text-fg no-underline">
              {c.footer.github}
            </a>
            <a href={MANUAL_URL} target="_blank" rel="noreferrer" className="text-faint hover:text-fg no-underline">
              {c.footer.manual}
            </a>
          </footer>
        )}
      </div>

      {/* Environments manager: a full-height right drawer overlaying the shell.
          It owns its own open→close animation (OverlayPresence), so it is
          always mounted and driven by `open`. */}
      <EnvironmentsDrawer open={envOpen} onClose={() => setEnvOpen(false)} />

      {/* The guided-tour overlay. Mounted here (the router root) so its finish
          step's react-router Link resolves; it renders nothing until active and
          portals itself to <body>. */}
      <TourOverlay />
    </div>
  );
}
