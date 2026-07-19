import { useEffect } from 'react';
import { Link, useLocation } from 'react-router-dom';
import { Eyebrow } from '@/components/layout/eyebrow';
import { Rich } from '@/components/content/rich';
import { useContent } from '@/i18n';

/**
 * Real 404 view for unmatched paths, rendered inside the `Shell` outlet so the
 * topbar/footer stay put. It names the missing path (from the live location)
 * and links home, instead of silently redirecting to `/` — a redirect hides
 * mistyped links and broken references, which this surfaces.
 */
export function NotFound() {
  const s = useContent().shell;
  const { pathname } = useLocation();

  useEffect(() => {
    document.title = s.notFoundMetaTitle;
  }, [s.notFoundMetaTitle]);

  return (
    <div className="min-h-[45vh] flex flex-col items-start justify-center gap-5">
      <Eyebrow>{s.notFoundEyebrow}</Eyebrow>
      <h1 className="font-display font-bold text-[clamp(28px,4.5vw,44px)] leading-[1.05] tracking-[-0.02em] text-fg">
        {s.notFoundTitle}
      </h1>
      <p className="text-[15px] leading-relaxed text-dim max-w-[56ch]">
        {/* `{path}` is wrapped in backticks in the string, so <Rich> renders the
            interpolated path as a mono chip. */}
        <Rich>{s.notFoundBody.replace('{path}', pathname)}</Rich>
      </p>
      <Link
        to="/"
        className="font-ui font-semibold text-[13.5px] bg-amber text-amber-ink rounded-control px-5 py-2.5 no-underline transition-colors hover:brightness-[1.06]"
      >
        {s.notFoundHome}
      </Link>
    </div>
  );
}
