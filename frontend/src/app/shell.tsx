import { useEffect, useState } from 'react';
import { Link, NavLink, Outlet } from 'react-router-dom';

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

  useEffect(() => {
    const handleScroll = () => {
      setCondensed((prev) => nextCondensed(prev, window.scrollY));
    };

    window.addEventListener('scroll', handleScroll, { passive: true });
    handleScroll(); // Initial scroll sync

    return () => {
      window.removeEventListener('scroll', handleScroll);
    };
  }, []);

  return (
    <div className="min-h-screen bg-bg text-fg font-ui flex flex-col">
      <div className="max-w-shell w-full mx-auto px-6 max-[480px]:px-4">
        {/* sticky topbar */}
        <div className="sticky top-0 z-40 bg-bg">
          <header
            className={`flex items-center gap-4 border-b border-line ${
              condensed ? 'h-[48px]' : 'h-[62px]'
            }`}
          >
            <Link
              to="/overview"
              className="font-display font-bold text-[19px] tracking-[0.01em] text-fg no-underline leading-none inline-block whitespace-nowrap flex-none"
            >
              F
              <span className="relative inline-block">
                K
                <span
                  style={{ boxShadow: '0 0 0 0.05em var(--bg)' }}
                  className="absolute left-[0.04em] top-[0.36em] w-[0.205em] h-[0.205em] rounded-full bg-amber"
                  aria-hidden="true"
                />
              </span>
              ST
            </Link>

            <nav className="flex gap-0.5">
              <NavLink to="/overview" className={navLinkClass}>
                Overview
              </NavLink>
              <NavLink to="/goals" className={navLinkClass}>
                Goals
              </NavLink>
              <NavLink to="/packages" className={navLinkClass}>
                Packages
              </NavLink>
            </nav>

            <div className="flex items-center gap-2 ml-auto">
              <div className="font-mono text-[11.5px] text-ghost border border-line bg-raise px-2 py-1 rounded-chip flex items-center gap-1.5 tabular-nums">
                <span className="w-1.5 h-1.5 rounded-full bg-ghost" />
                <span>github — unknown</span>
              </div>

              <NavLink
                to="/settings"
                title="Sign-in pending (NyxID)"
                aria-label="Settings — sign-in pending (NyxID)"
                className={({ isActive }) =>
                  `w-[30px] h-[30px] rounded-full bg-raise-2 border flex items-center justify-center text-dim font-semibold text-[11px] tracking-[0.02em] no-underline cursor-pointer transition-colors ${
                    isActive
                      ? 'border-amber text-amber'
                      : 'border-line-2 hover:border-faint hover:text-fg'
                  }`
                }
              >
                –
              </NavLink>
            </div>
          </header>
        </div>

        {/* main content */}
        <main className="py-6">
          <Outlet />
        </main>
      </div>
    </div>
  );
}
