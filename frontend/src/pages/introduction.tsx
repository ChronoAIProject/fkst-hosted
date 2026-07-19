import { useEffect } from 'react';
import type { CSSProperties } from 'react';
import { Link } from 'react-router-dom';
import { Eyebrow } from '@/components/layout/eyebrow';
import { FkstMark } from '@/components/brand/fkst-mark';
import { Rich } from '@/components/content/rich';
import { useContent } from '@/i18n';
import { MANUAL_URL, MENTAL_ORDER, FLOW_ORDER } from '@/i18n/literals';

/** Per-index entrance delay for `.anim-row-in` (reduced-motion-safe — the class
 *  is disabled under `prefers-reduced-motion`, leaving rows at rest). */
const stagger = (i: number): CSSProperties => ({ ['--stagger']: `${i * 70}ms` } as CSSProperties);

/** The marketing hero's abstract right-side accent: concentric amber hairline
 *  rings around a floating, breathing gradient orb, with a small glass chip
 *  cluster. Purely decorative (aria-hidden), clipped to its own box so it can
 *  never introduce window/horizontal scroll, and every motion utility collapses
 *  to a static state under reduced motion. */
function HeroAccent() {
  return (
    <div
      aria-hidden="true"
      className="relative hidden lg:block min-h-[320px] overflow-hidden rounded-panel"
    >
      {/* Concentric hairline rings (transparent centers), expanding outward. The
          outer ring breathes with the shared amber glow-pulse. */}
      <div className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 w-[360px] h-[360px] rounded-full border border-[color-mix(in_oklab,var(--amber)_22%,transparent)] anim-glow-pulse" />
      <div className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 w-[260px] h-[260px] rounded-full border border-[color-mix(in_oklab,var(--gold)_24%,transparent)]" />
      <div className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 w-[168px] h-[168px] rounded-full border border-[color-mix(in_oklab,var(--amber)_30%,transparent)]" />

      {/* Soft amber bloom washing the whole accent — pure paint, fades to
          transparent well inside the clipped box. */}
      <div
        className="absolute inset-0"
        style={{
          background:
            'radial-gradient(60% 60% at 50% 42%, color-mix(in oklab, var(--amber) 22%, transparent), transparent 72%)',
        }}
      />

      {/* Central glowing gradient orb, gently bobbing. */}
      <div className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 anim-float">
        <div className="w-[92px] h-[92px] rounded-full bg-grad-accent anim-gradient-shift shadow-[var(--shadow-2),var(--glow-amber)]" />
      </div>

      {/* Floating glass chip cluster — abstract, decorative. */}
      <div className="absolute left-[14%] top-[18%] bg-glass backdrop-blur-glass border border-line-2 rounded-card px-3 py-2 shadow-[var(--shadow-2),var(--highlight-top)] anim-float [animation-delay:600ms]">
        <div className="flex items-center gap-2">
          <span className="w-1.5 h-1.5 rounded-full bg-green shadow-glow-green anim-glow-pulse" />
          <span className="w-14 h-1.5 rounded-full bg-[color-mix(in_oklab,var(--fg)_16%,transparent)]" />
        </div>
        <span className="mt-2 block w-9 h-1.5 rounded-full bg-[color-mix(in_oklab,var(--amber)_45%,transparent)]" />
      </div>
      <div className="absolute right-[12%] bottom-[16%] bg-glass backdrop-blur-glass border border-line-2 rounded-card px-3 py-2 shadow-[var(--shadow-2),var(--highlight-top)] anim-float [animation-delay:1200ms]">
        <span className="block w-10 h-1.5 rounded-full bg-[color-mix(in_oklab,var(--fg)_16%,transparent)]" />
        <span className="mt-2 block w-16 h-1.5 rounded-full bg-[color-mix(in_oklab,var(--fg)_10%,transparent)]" />
      </div>
    </div>
  );
}

/** The hero headline: a bright fg→dim display sweep across the whole line, with
 *  the clause after the last comma lifted into the amber→gold brand gradient.
 *  Splitting is language-tolerant — a title with no comma (e.g. zh) renders as a
 *  single fg-gradient span. Both spans concatenate to the exact source string,
 *  so the accessible heading name is unchanged. */
function HeroTitle({ title }: { title: string }) {
  const ci = title.lastIndexOf(',');
  const head = ci >= 0 ? title.slice(0, ci + 1) : title;
  const accent = ci >= 0 ? title.slice(ci + 1) : '';
  return (
    <h1 className="mt-5 font-display text-display-xl text-fg max-w-[16ch]">
      <span className="grad-text grad-text-fg anim-gradient-shift">{head}</span>
      {accent && <span className="grad-text anim-gradient-shift">{accent}</span>}
    </h1>
  );
}

export function Introduction() {
  const c = useContent();

  useEffect(() => {
    document.title = c.intro.metaTitle;
  }, [c.intro.metaTitle]);

  return (
    <div className="flex flex-col gap-20 max-[600px]:gap-16">
      {/* Hero */}
      <section className="relative overflow-hidden pt-4 grid grid-cols-1 lg:grid-cols-[1.15fr_0.85fr] gap-x-10 items-center">
        {/* Soft radial glow anchored behind the headline. */}
        <div
          aria-hidden="true"
          className="pointer-events-none absolute -left-24 -top-16 w-[520px] h-[420px] opacity-80"
          style={{
            background:
              'radial-gradient(45% 45% at 30% 40%, color-mix(in oklab, var(--amber) 16%, transparent), transparent 70%)',
          }}
        />
        <div className="relative">
          <div className="anim-row-in" style={stagger(0)}>
            <Eyebrow>{c.intro.eyebrow}</Eyebrow>
          </div>
          <div className="anim-row-in" style={stagger(1)}>
            <HeroTitle title={c.intro.heroTitle} />
          </div>
          <p className="mt-6 text-[16px] leading-relaxed text-dim max-w-[62ch] anim-row-in" style={stagger(2)}>
            <FkstMark className="text-[16px] text-fg" /> <Rich>{c.intro.heroLede}</Rich>
          </p>
          <div className="mt-8 flex items-center gap-3 flex-wrap anim-row-in" style={stagger(3)}>
            <Link
              to="/get-started"
              className="anim-sheen font-ui font-semibold text-[13.5px] bg-grad-accent text-amber-ink rounded-control px-5 py-2.5 no-underline shadow-[var(--shadow-2),var(--glow-amber)] transition-[filter,box-shadow] hover:brightness-110"
            >
              {c.intro.ctaStart}
            </Link>
            <a
              href={MANUAL_URL}
              target="_blank"
              rel="noreferrer"
              className="bg-glass backdrop-blur-glass border border-line-2 font-ui font-medium text-[13.5px] text-dim rounded-control px-5 py-2.5 no-underline shadow-2 transition-[color,border-color,box-shadow] hover:text-fg hover:border-faint hover:shadow-glow-amber"
            >
              {c.intro.ctaManual}
            </a>
          </div>
        </div>

        <HeroAccent />
      </section>

      {/* What is FKST */}
      <section>
        <Eyebrow>{c.intro.whatIsEyebrow}</Eyebrow>
        <div className="mt-5 grid grid-cols-[1.1fr_0.9fr] gap-x-12 gap-y-6 max-[820px]:grid-cols-1">
          <h2 className="grad-text grad-text-fg font-display text-display-md font-semibold anim-row-in" style={stagger(0)}>
            {c.intro.whatIsTitle}
          </h2>
          <div className="flex flex-col gap-4 text-[14px] leading-relaxed text-dim anim-row-in" style={stagger(1)}>
            {c.intro.whatIsBody.map((para, i) => (
              <p key={i}>
                <Rich>{para}</Rich>
              </p>
            ))}
          </div>
        </div>

        {/* One-line thesis — glass callout with an amber left accent + bloom. */}
        <div className="mt-10 anim-row-in bg-glass backdrop-blur-glass border border-line border-l-2 border-l-amber rounded-card px-5 py-4 shadow-[var(--shadow-2),var(--glow-amber)]" style={stagger(2)}>
          <p className="text-[15px] leading-relaxed text-fg">
            <Rich>{c.intro.thesis}</Rich>
          </p>
        </div>
      </section>

      {/* Mental model */}
      <section>
        <Eyebrow>{c.intro.mentalEyebrow}</Eyebrow>
        <div className="mt-5 grid grid-cols-3 gap-px bg-line border border-line rounded-panel overflow-hidden shadow-2 max-[720px]:grid-cols-1">
          {MENTAL_ORDER.map((key, i) => {
            const m = c.intro.mental[key];
            return (
              <div
                key={key}
                className="bg-glass backdrop-blur-glass p-5 flex flex-col gap-2.5 transition-colors hover:bg-glass-2 anim-row-in"
                style={stagger(i)}
              >
                <span className="font-display font-semibold text-[15px] tracking-[0.01em] grad-text grad-text-fg">
                  {m.term}
                </span>
                <span className="text-[13px] leading-relaxed text-dim">
                  <Rich>{m.is}</Rich>
                </span>
                <span className="font-mono text-[11.5px] leading-relaxed text-ghost mt-auto pt-1">
                  <Rich>{m.control}</Rich>
                </span>
              </div>
            );
          })}
        </div>
      </section>

      {/* What the hosted service provides */}
      <section>
        <Eyebrow>{c.intro.providesEyebrow}</Eyebrow>
        <h2 className="mt-5 grad-text grad-text-fg font-display text-display-md font-semibold max-w-[24ch]">
          {c.intro.providesTitle}
        </h2>
        <div className="mt-8 grid grid-cols-3 gap-4 max-[900px]:grid-cols-2 max-[600px]:grid-cols-1">
          {c.intro.features.map((f, i) => (
            <div
              key={f.title}
              className="grad-border hover-lift rounded-card p-5 flex flex-col gap-2.5 shadow-[var(--shadow-2),var(--highlight-top)] anim-row-in"
              style={stagger(i)}
            >
              <span
                className="w-1.5 h-1.5 rounded-full bg-amber shadow-glow-amber anim-glow-pulse"
                aria-hidden="true"
              />
              <h3 className="font-ui font-semibold text-[14.5px] text-fg leading-snug">{f.title}</h3>
              <p className="text-[13px] leading-relaxed text-dim">{f.body}</p>
            </div>
          ))}
        </div>
      </section>

      {/* Flow */}
      <section>
        <Eyebrow>{c.intro.flowEyebrow}</Eyebrow>
        <div className="mt-6 flex items-stretch gap-0 max-[760px]:flex-col">
          {FLOW_ORDER.map((key, idx) => {
            const step = c.intro.flow[key];
            return (
              <div key={key} className="flex items-stretch flex-1 min-w-0 max-[760px]:flex-none">
                <div
                  className="flex-1 min-w-0 grad-border hover-lift rounded-card px-4 py-4 flex flex-col justify-center gap-1 shadow-2 anim-row-in"
                  style={stagger(idx)}
                >
                  <span className="font-mono text-[10.5px] grad-text grad-text-fg tabular-nums">
                    {String(idx + 1).padStart(2, '0')}
                  </span>
                  <span className="font-display font-semibold text-[14px] text-fg leading-tight">
                    {step.label}
                  </span>
                  <span className="font-mono text-[11px] text-ghost">{step.sub}</span>
                </div>
                {idx < FLOW_ORDER.length - 1 && (
                  <div
                    className="flex items-center justify-center px-2 text-amber flex-none max-[760px]:h-4 max-[760px]:rotate-90 max-[760px]:self-center"
                    aria-hidden="true"
                  >
                    →
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </section>

      {/* CTA band */}
      <section className="relative overflow-hidden bg-glass backdrop-blur-glass border border-line-2 rounded-panel px-8 py-10 max-[600px]:px-5 max-[600px]:py-8 flex items-center justify-between gap-6 flex-wrap shadow-[var(--shadow-glow),var(--highlight-top)]">
        {/* Warm bloom drifting behind the band's copy. */}
        <div
          aria-hidden="true"
          className="pointer-events-none absolute -right-16 -top-16 w-[360px] h-[320px] opacity-70"
          style={{
            background:
              'radial-gradient(50% 50% at 60% 40%, color-mix(in oklab, var(--amber) 16%, transparent), transparent 70%)',
          }}
        />
        <div className="relative">
          <h2 className="grad-text grad-text-fg font-display text-display-md font-semibold">
            {c.intro.ctaTitle}
          </h2>
          <p className="mt-2 text-[14px] text-dim max-w-[52ch]">{c.intro.ctaBody}</p>
        </div>
        <Link
          to="/get-started"
          className="relative anim-sheen font-ui font-semibold text-[13.5px] bg-grad-accent text-amber-ink rounded-control px-5 py-2.5 no-underline shadow-[var(--shadow-2),var(--glow-amber)] transition-[filter,box-shadow] hover:brightness-110 flex-none"
        >
          {c.intro.ctaButton}
        </Link>
      </section>
    </div>
  );
}
