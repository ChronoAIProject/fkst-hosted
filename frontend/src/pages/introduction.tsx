import { useEffect } from 'react';
import { Link } from 'react-router-dom';
import { Eyebrow } from '@/components/layout/eyebrow';
import { FkstMark } from '@/components/brand/fkst-mark';
import { Rich } from '@/components/content/rich';
import { useContent } from '@/i18n';
import { MANUAL_URL, MENTAL_ORDER, FLOW_ORDER } from '@/i18n/literals';

export function Introduction() {
  const c = useContent();

  useEffect(() => {
    document.title = c.intro.metaTitle;
  }, [c.intro.metaTitle]);

  return (
    <div className="flex flex-col gap-20 max-[600px]:gap-16">
      {/* Hero */}
      <section className="pt-4">
        <Eyebrow>{c.intro.eyebrow}</Eyebrow>
        <h1 className="mt-5 font-display font-bold text-[clamp(30px,5vw,52px)] leading-[1.05] tracking-[-0.02em] text-fg max-w-[16ch]">
          {c.intro.heroTitle}
        </h1>
        <p className="mt-6 text-[16px] leading-relaxed text-dim max-w-[62ch]">
          <FkstMark className="text-[16px] text-fg" /> <Rich>{c.intro.heroLede}</Rich>
        </p>
        <div className="mt-8 flex items-center gap-3 flex-wrap">
          <Link
            to="/get-started"
            className="font-ui font-semibold text-[13.5px] bg-amber text-amber-ink rounded-control px-5 py-2.5 no-underline transition-colors hover:brightness-[1.06]"
          >
            {c.intro.ctaStart}
          </Link>
          <a
            href={MANUAL_URL}
            target="_blank"
            rel="noreferrer"
            className="font-ui font-medium text-[13.5px] text-dim border border-line-2 bg-raise rounded-control px-5 py-2.5 no-underline transition-colors hover:text-fg hover:border-faint"
          >
            {c.intro.ctaManual}
          </a>
        </div>
      </section>

      {/* What is FKST */}
      <section>
        <Eyebrow>{c.intro.whatIsEyebrow}</Eyebrow>
        <div className="mt-5 grid grid-cols-[1.1fr_0.9fr] gap-x-12 gap-y-6 max-[820px]:grid-cols-1">
          <h2 className="font-display font-semibold text-[24px] leading-[1.2] tracking-[-0.01em] text-fg">
            {c.intro.whatIsTitle}
          </h2>
          <div className="flex flex-col gap-4 text-[14px] leading-relaxed text-dim">
            {c.intro.whatIsBody.map((para, i) => (
              <p key={i}>
                <Rich>{para}</Rich>
              </p>
            ))}
          </div>
        </div>

        {/* One-line thesis */}
        <div className="mt-10 border border-line border-l-2 border-l-amber rounded-card bg-[color-mix(in_oklab,var(--raise)_55%,transparent)] px-5 py-4">
          <p className="text-[15px] leading-relaxed text-fg">
            <Rich>{c.intro.thesis}</Rich>
          </p>
        </div>
      </section>

      {/* Mental model */}
      <section>
        <Eyebrow>{c.intro.mentalEyebrow}</Eyebrow>
        <div className="mt-5 grid grid-cols-3 gap-px bg-line border border-line rounded-panel overflow-hidden max-[720px]:grid-cols-1">
          {MENTAL_ORDER.map((key) => {
            const m = c.intro.mental[key];
            return (
              <div key={key} className="bg-raise p-5 flex flex-col gap-2.5">
                <span className="font-display font-semibold text-[15px] tracking-[0.01em] text-fg">
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
        <h2 className="mt-5 font-display font-semibold text-[24px] leading-[1.2] tracking-[-0.01em] text-fg max-w-[24ch]">
          {c.intro.providesTitle}
        </h2>
        <div className="mt-8 grid grid-cols-3 gap-4 max-[900px]:grid-cols-2 max-[600px]:grid-cols-1">
          {c.intro.features.map((f) => (
            <div
              key={f.title}
              className="border border-line rounded-card bg-raise p-5 flex flex-col gap-2.5 transition-colors hover:border-line-2"
            >
              <span className="w-1.5 h-1.5 rounded-full bg-amber" aria-hidden="true" />
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
                <div className="flex-1 min-w-0 border border-line rounded-card bg-raise px-4 py-4 flex flex-col justify-center gap-1">
                  <span className="font-mono text-[10.5px] text-ghost tabular-nums">
                    {String(idx + 1).padStart(2, '0')}
                  </span>
                  <span className="font-display font-semibold text-[14px] text-fg leading-tight">
                    {step.label}
                  </span>
                  <span className="font-mono text-[11px] text-ghost">{step.sub}</span>
                </div>
                {idx < FLOW_ORDER.length - 1 && (
                  <div
                    className="flex items-center justify-center px-2 text-ghost flex-none max-[760px]:h-4 max-[760px]:rotate-90 max-[760px]:self-center"
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
      <section className="border border-line rounded-panel bg-[color-mix(in_oklab,var(--raise)_60%,transparent)] px-8 py-10 max-[600px]:px-5 max-[600px]:py-8 flex items-center justify-between gap-6 flex-wrap">
        <div>
          <h2 className="font-display font-semibold text-[22px] tracking-[-0.01em] text-fg">
            {c.intro.ctaTitle}
          </h2>
          <p className="mt-2 text-[14px] text-dim max-w-[52ch]">{c.intro.ctaBody}</p>
        </div>
        <Link
          to="/get-started"
          className="font-ui font-semibold text-[13.5px] bg-amber text-amber-ink rounded-control px-5 py-2.5 no-underline transition-colors hover:brightness-[1.06] flex-none"
        >
          {c.intro.ctaButton}
        </Link>
      </section>
    </div>
  );
}
