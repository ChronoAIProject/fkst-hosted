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
    <div className="min-h-[45vh] flex flex-col items-center justify-center">
      {/* Hero-accent card: opaque --raise fill behind an amber->gold hairline
          (.grad-border-accent), lifted by card depth + a soft amber bloom and an
          inner top highlight — the premium accent-surface recipe. */}
      <div className="anim-row-in grad-border grad-border-accent rounded-panel p-8 max-[600px]:p-6 flex flex-col items-start gap-5 max-w-[560px] w-full shadow-[var(--shadow-2),var(--glow-amber)] shadow-highlight-top">
        <Eyebrow>{s.notFoundEyebrow}</Eyebrow>
        {/* Display headline as a bright fg->dim gradient sweep (legible low end). */}
        <h1 className="grad-text grad-text-fg font-display font-bold text-display-lg leading-[1.05] tracking-[-0.02em]">
          {s.notFoundTitle}
        </h1>
        <p className="text-[15px] leading-relaxed text-dim max-w-[56ch]">
          {/* `{path}` is wrapped in backticks in the string, so <Rich> renders the
              interpolated path as a mono chip. */}
          <Rich>{s.notFoundBody.replace('{path}', pathname)}</Rich>
        </p>
        {/* Primary CTA: brand gradient fill + amber bloom, with a one-shot sheen. */}
        <Link
          to="/"
          className="anim-sheen relative overflow-hidden font-ui font-semibold text-[13.5px] bg-grad-accent text-amber-ink rounded-control px-5 py-2.5 no-underline transition-[filter] hover:brightness-110 shadow-[var(--shadow-2),var(--glow-amber)]"
        >
          {s.notFoundHome}
        </Link>
      </div>
    </div>
  );
}
