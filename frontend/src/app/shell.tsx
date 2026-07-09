import { useEffect, useState } from 'react';
import { Link, NavLink, Outlet } from 'react-router-dom';
import { FkstMark } from '../components/brand/fkst-mark';
import { LanguageToggle } from '../components/layout/language-toggle';
import { useContent } from '../i18n';
import { REPO, MANUAL_URL } from '../i18n/literals';

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

  useEffect(() => {
    const handleScroll = () => {
      setCondensed((prev) => nextCondensed(prev, window.scrollY));
    };
    window.addEventListener('scroll', handleScroll, { passive: true });
    handleScroll(); // initial sync
    return () => window.removeEventListener('scroll', handleScroll);
  }, []);

  return (
    <div className="min-h-screen bg-bg text-fg font-ui flex flex-col">
      <div className="max-w-shell w-full mx-auto px-6 max-[480px]:px-4 flex-1 flex flex-col">
        {/* sticky topbar */}
        <div className="sticky top-0 z-40 bg-bg">
          <header
            className={`flex items-center gap-4 border-b border-line ${
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
            </nav>

            <div className="flex items-center gap-2 ml-auto">
              <LanguageToggle />
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

        {/* main content */}
        <main className="py-10 max-[480px]:py-8 flex-1">
          <Outlet />
        </main>

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
      </div>
    </div>
  );
}
