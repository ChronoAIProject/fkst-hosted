import { useEffect, useRef, useState } from 'react';
import { Link, NavLink, Outlet, useLocation } from 'react-router-dom';
import { FkstMark } from '../components/brand/fkst-mark';
import { LanguageToggle } from '../components/layout/language-toggle';
import { RouteTransition } from '../components/ui/motion';
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

const navLinkClass = ({ isActive }: { isActive: boolean }) =>
  `text-[13.5px] no-underline px-3 py-[7px] rounded-control transition-colors ${
    isActive
      ? 'text-fg bg-raise hover:bg-raise-2'
      : 'text-faint hover:text-dim hover:bg-[color-mix(in_oklab,var(--raise)_55%,transparent)]'
  }`;

export function Shell() {
  const [condensed, setCondensed] = useState(false);
  const c = useContent();
  const { isAuthenticated, signOut } = useAuth();
  const location = useLocation();
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

  return (
    <div className="h-[100dvh] bg-bg text-fg font-ui flex flex-col overflow-hidden">
      <div className="max-w-shell w-full mx-auto px-6 max-[480px]:px-4 flex-1 min-h-0 flex flex-col">
        {/* pinned topbar (the column itself never scrolls, so no sticky needed) */}
        <div className="flex-none z-40 bg-bg">
          <header
            className={`flex items-center gap-4 border-b border-line transition-[height] duration-200 motion-reduce:transition-none ${
              condensed ? 'h-[48px]' : 'h-[62px]'
            }`}
          >
            <Link
              to="/"
              className="text-fg no-underline inline-block flex-none"
              aria-label={c.nav.homeAria}
            >
              <FkstMark className="text-[19px]" />
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
              {isAuthenticated && (
                <button
                  type="button"
                  onClick={signOut}
                  className="font-mono text-[12px] text-faint hover:text-fg px-2.5 py-[7px] rounded-control transition-colors cursor-pointer max-[600px]:hidden"
                >
                  {c.auth.signOut}
                </button>
              )}
              <a
                href={REPO}
                target="_blank"
                rel="noreferrer"
                className="font-mono text-[12px] text-faint hover:text-fg no-underline px-2.5 py-[7px] rounded-control transition-colors max-[720px]:hidden"
              >
                GitHub ↗
              </a>
              <NavLink
                to="/get-started"
                className="font-ui font-semibold text-[12.5px] bg-amber text-amber-ink rounded-control px-3.5 py-[7px] flex-none no-underline transition-colors hover:brightness-[1.06] max-[480px]:hidden"
              >
                {c.nav.getStartedCta}
              </NavLink>
            </div>
          </header>
        </div>

        {/* The SOLE scroll container: topbar stays pinned above; both the routed
            content and the footer scroll together inside here (the footer keeps
            its end-of-document position). */}
        <main ref={mainRef} className="flex-1 min-h-0 overflow-y-auto">
          {/* Keyed on the pathname so a route change crossfades on the shared
              curve; collapses to an instant swap under reduced motion. */}
          <RouteTransition k={location.pathname}>
            <div className="py-10 max-[480px]:py-8">
              <Outlet />
            </div>
          </RouteTransition>

          {/* footer */}
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
        </main>
      </div>
    </div>
  );
}
