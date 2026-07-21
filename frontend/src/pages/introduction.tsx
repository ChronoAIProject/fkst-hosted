import { useEffect } from 'react';
import { Link } from 'react-router-dom';
import { useContent } from '@/i18n';
import { MANUAL_URL } from '@/i18n/literals';

/** One 44px connector of the hero flow-line: a hairline with a traveling accent
 *  dot. The second connector's dot is phase-offset so the two alternate. The
 *  connector (not the flow-line's text labels) is the decorative part, so the
 *  aria-hidden sits here; the dot rests invisible under reduced motion, leaving
 *  the labels + hairlines as a static diagram. */
function PipeConnector({ delayed = false }: { delayed?: boolean }) {
  return (
    <span aria-hidden="true" className="relative w-11 h-px bg-line-2 flex-none">
      <span
        className={`absolute left-0 top-[-2.5px] w-1.5 h-1.5 rounded-full bg-amber anim-beam-dot ${
          delayed ? '[animation-delay:1.2s]' : ''
        }`}
      />
    </span>
  );
}

/** The v2 landing: a single-viewport centered hero — eyebrow, two-line gradient
 *  headline, one-line lede, CTA pair, and the trigger→session→PR flow-line —
 *  over a masked line-grid and an accent glow blob. The shell renders this
 *  route full-height with the pinned footer, so the page never scrolls. */
export function Introduction() {
  const c = useContent();

  useEffect(() => {
    document.title = c.intro.metaTitle;
  }, [c.intro.metaTitle]);

  return (
    <section className="relative h-full flex flex-col items-center justify-center text-center overflow-hidden px-6 pt-5 pb-4 max-[480px]:px-4">
      {/* Background layer 1 — an 80px line grid, radially masked so it fades
          out well before the edges. Pure paint. */}
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-0 opacity-40"
        style={{
          backgroundImage:
            'linear-gradient(color-mix(in oklab, var(--line) 55%, transparent) 1px, transparent 1px), linear-gradient(90deg, color-mix(in oklab, var(--line) 55%, transparent) 1px, transparent 1px)',
          backgroundSize: '80px 80px',
          WebkitMaskImage: 'radial-gradient(70% 55% at 50% 26%, black, transparent 78%)',
          maskImage: 'radial-gradient(70% 55% at 50% 26%, black, transparent 78%)',
        }}
      />
      {/* Background layer 2 — the accent glow blob washing down from above the
          fold. Clipped by the section's overflow-hidden. */}
      <div
        aria-hidden="true"
        className="pointer-events-none absolute left-1/2 top-[-300px] w-[900px] h-[520px] -translate-x-1/2"
        style={{
          background:
            'radial-gradient(50% 60% at 50% 48%, color-mix(in oklab, var(--amber) 15%, transparent), transparent 70%)',
        }}
      />

      <div className="relative flex flex-col items-center">
        {/* faint (not ghost): 10px text must clear WCAG AA — ghost is 3.6:1. */}
        <span className="font-mono text-[10px] tracking-[0.2em] uppercase text-faint">
          {c.intro.eyebrow}
        </span>

        <h1 className="mt-[18px] font-display font-bold leading-[1.04] tracking-[-0.045em] text-[length:clamp(36px,4.4vw,58px)] [@media(max-height:760px)]:text-[length:clamp(26px,3.4vw,38px)] max-w-[16ch]">
          {/* Hero-specific gradients per the comp: the top line holds pure fg
              through 30% then sinks toward --faint; the accent line loops a
              3-stop bright-tier sweep so both hues stay visible as it shimmers
              (unlike the app-wide .grad-text/-fg recipes). */}
          <span
            className="grad-text-fg"
            style={{
              backgroundImage:
                'linear-gradient(180deg, var(--fg) 30%, color-mix(in oklab, var(--fg) 55%, var(--faint)))',
            }}
          >
            {c.intro.heroTitleTop}
          </span>
          <br />
          <span
            className="grad-text anim-gradient-loop"
            style={{
              backgroundImage:
                'linear-gradient(100deg, var(--amber), var(--gold) 60%, var(--amber))',
            }}
          >
            {c.intro.heroTitleAccent}
          </span>
        </h1>

        <p className="mt-[18px] text-[15px] leading-[1.6] text-dim max-w-[44ch]">
          {c.intro.heroLede}
        </p>

        <div className="mt-[26px] flex items-center justify-center gap-[14px] flex-wrap">
          <Link
            to="/get-started"
            className="bg-fg text-bg rounded-pill px-[26px] py-[11px] font-ui font-semibold text-[13.5px] no-underline transition-opacity duration-150 hover:opacity-85"
          >
            {c.intro.ctaStart}
          </Link>
          <a
            href={MANUAL_URL}
            target="_blank"
            rel="noreferrer"
            className="text-[13px] font-medium text-faint hover:text-fg no-underline transition-colors"
          >
            {c.intro.ctaManual}
          </a>
        </div>

        {/* The flow-line: trigger issue ─●→ live session ─●→ a PR per task.
            The text labels stay in the accessibility tree (they carry real
            information); only the connectors and dots are decorative. Hidden
            entirely on very short viewports so the hero always fits. */}
        <div className="mt-11 flex items-center gap-[14px] font-mono text-[11.5px] text-faint [@media(max-height:560px)]:hidden">
          <span>{c.intro.pipeTrigger}</span>
          <PipeConnector />
          <span className="inline-flex items-center gap-[7px] text-dim">
            <span
              aria-hidden="true"
              className="w-[5px] h-[5px] rounded-full bg-green anim-dot-blink [animation-duration:1.6s] flex-none"
            />
            {c.intro.pipeSession}
          </span>
          <PipeConnector delayed />
          <span>{c.intro.pipeWork}</span>
        </div>
      </div>
    </section>
  );
}
