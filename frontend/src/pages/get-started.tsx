import React, { useEffect } from 'react';
import { cn } from '@/lib/utils';
import { Eyebrow } from '@/components/layout/eyebrow';
import { CodeBlock } from '@/components/content/code-block';
import { Callout } from '@/components/content/callout';
import { Rich } from '@/components/content/rich';
import { useContent } from '@/i18n';
import {
  MANUAL_URL,
  PKG_GRAMMAR,
  PKG_REF_EXAMPLE,
  TRIGGER_EXAMPLE,
  GH_CREATE,
  CURL_LOGS,
  TRIGGER_FIELDS,
  GRAMMAR_PARTS,
  SIGNALS,
  STEP_ORDER,
  type SignalTone,
} from '@/i18n/literals';

const DOT_TONE: Record<SignalTone, string> = {
  green: 'bg-green',
  neutral: 'bg-ghost',
  amber: 'bg-amber',
  red: 'bg-red',
};

function Step({
  word,
  n,
  id,
  title,
  children,
}: {
  word: string;
  n: number;
  id: string;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section id={id} className="scroll-mt-[80px]">
      <Eyebrow>
        {word} {n}
      </Eyebrow>
      <h2 className="mt-4 font-display font-semibold text-[22px] leading-[1.2] tracking-[-0.01em] text-fg">
        {title}
      </h2>
      <div className="mt-5 flex flex-col gap-5">{children}</div>
    </section>
  );
}

function P({ children }: { children: string }) {
  return (
    <p className="text-[14px] leading-relaxed text-dim max-w-[68ch]">
      <Rich>{children}</Rich>
    </p>
  );
}

export function GetStarted() {
  const c = useContent();
  const gs = c.gs;

  useEffect(() => {
    document.title = gs.metaTitle;
  }, [gs.metaTitle]);

  return (
    <div className="flex flex-col gap-16 max-w-[880px]">
      {/* Header */}
      <header>
        <Eyebrow>{gs.eyebrow}</Eyebrow>
        <h1 className="mt-5 font-display font-bold text-[clamp(28px,4vw,40px)] leading-[1.1] tracking-[-0.02em] text-fg">
          {gs.title}
        </h1>
        <p className="mt-5 text-[15px] leading-relaxed text-dim max-w-[68ch]">{gs.lede}</p>

        {/* Step index */}
        <nav className="mt-8 border border-line rounded-panel bg-raise overflow-hidden">
          <ol className="grid grid-cols-2 max-[640px]:grid-cols-1 gap-px bg-line">
            {STEP_ORDER.map((id, i) => (
              <li key={id} className="bg-raise">
                <a
                  href={`#${id}`}
                  className="flex items-baseline gap-3 px-4 py-3 no-underline group hover:bg-raise-2 transition-colors"
                >
                  <span className="font-mono text-[11px] text-ghost tabular-nums flex-none">
                    {String(i + 1).padStart(2, '0')}
                  </span>
                  <span className="text-[13.5px] text-dim group-hover:text-fg transition-colors">
                    {gs.stepTitles[id]}
                  </span>
                </a>
              </li>
            ))}
          </ol>
        </nav>
      </header>

      {/* Step 1 — Install */}
      <Step word={gs.stepWord} n={1} id="install" title={gs.stepTitles.install}>
        <P>{gs.install.body}</P>
        <Callout tone="warn" title={gs.install.calloutTitle}>
          <Rich>{gs.install.callout}</Rich>
        </Callout>
      </Step>

      {/* Step 2 — Start a session */}
      <Step word={gs.stepWord} n={2} id="start" title={gs.stepTitles.start}>
        <P>{gs.start.body}</P>
        <CodeBlock caption={gs.start.exampleCaption} code={TRIGGER_EXAMPLE} />
        <P>{gs.start.createIntro}</P>
        <CodeBlock caption={gs.start.terminalCaption} code={GH_CREATE} />
        <Callout tone="warn" title={gs.start.calloutTitle}>
          <Rich>{gs.start.callout}</Rich>
        </Callout>
      </Step>

      {/* Step 3 — Parameters */}
      <Step word={gs.stepWord} n={3} id="parameters" title={gs.stepTitles.parameters}>
        <P>{gs.parameters.intro}</P>
        <div className="border border-line rounded-panel overflow-hidden flex flex-col gap-px bg-line">
          {TRIGGER_FIELDS.map((f) => (
            <div
              key={f.key}
              className="bg-raise p-4 grid grid-cols-[minmax(180px,220px)_1fr] gap-4 max-[620px]:grid-cols-1 max-[620px]:gap-2"
            >
              <div className="flex flex-col gap-2 min-w-0">
                <code className="font-mono text-[12.5px] text-fg break-words">{f.name}</code>
                <span
                  className={cn(
                    'font-mono text-[10px] uppercase tracking-[0.1em] font-semibold w-fit px-1.5 py-0.5 rounded-chip border',
                    f.required
                      ? 'text-amber border-[color-mix(in_oklab,var(--amber)_40%,var(--line))]'
                      : 'text-ghost border-line-2'
                  )}
                >
                  {f.required ? gs.requiredLabel : gs.optionalLabel}
                </span>
              </div>
              <p className="text-[13px] leading-relaxed text-dim min-w-0">
                <Rich>{gs.parameters.fieldRules[f.key]}</Rich>
              </p>
            </div>
          ))}
        </div>
        <Callout tone="note" title={gs.parameters.calloutTitle}>
          <Rich>{gs.parameters.callout}</Rich>
        </Callout>
      </Step>

      {/* Step 4 — Package refs */}
      <Step
        word={gs.stepWord}
        n={4}
        id="packages"
        title={`${gs.stepTitles.packages} — ${PKG_GRAMMAR}`}
      >
        <P>{gs.packages.intro}</P>
        <div className="grid grid-cols-3 gap-px bg-line border border-line rounded-panel overflow-hidden max-[620px]:grid-cols-1">
          {GRAMMAR_PARTS.map((g) => (
            <div key={g.key} className="bg-raise p-4 flex flex-col gap-2">
              <code className="font-mono text-[12.5px] text-fg">{g.part}</code>
              <span className="text-[12.5px] leading-relaxed text-dim">
                <Rich>{gs.packages.grammar[g.key]}</Rich>
              </span>
            </div>
          ))}
        </div>
        <CodeBlock caption={gs.packages.exampleCaption} code={PKG_REF_EXAMPLE} />
      </Step>

      {/* Step 5 — Queue work */}
      <Step word={gs.stepWord} n={5} id="queue" title={gs.stepTitles.queue}>
        <P>{gs.queue.body}</P>
        <Callout tone="tip" title={gs.queue.calloutTitle}>
          <Rich>{gs.queue.callout}</Rich>
        </Callout>
      </Step>

      {/* Step 6 — Status */}
      <Step word={gs.stepWord} n={6} id="status" title={gs.stepTitles.status}>
        <P>{gs.status.intro}</P>
        <div className="border border-line rounded-panel overflow-hidden flex flex-col gap-px bg-line">
          {SIGNALS.map((s) => (
            <div
              key={s.key}
              className="bg-raise p-4 grid grid-cols-[minmax(0,1.1fr)_minmax(0,1.4fr)] gap-4 items-start max-[620px]:grid-cols-1 max-[620px]:gap-2"
            >
              <div className="flex items-center gap-2.5 min-w-0">
                <span
                  className={cn('w-2 h-2 rounded-full flex-none', DOT_TONE[s.tone])}
                  aria-hidden="true"
                />
                <code className="font-mono text-[12.5px] text-fg truncate min-w-0">{s.name}</code>
                <span className="font-mono text-[10px] uppercase tracking-[0.08em] text-ghost border border-line-2 rounded-chip px-1.5 py-0.5 flex-none">
                  {gs.status.kind[s.key]}
                </span>
              </div>
              <div className="min-w-0">
                <span className="font-mono text-[11px] text-ghost">
                  {gs.status.onWord} {gs.status.where[s.key]}
                </span>
                <p className="text-[13px] leading-relaxed text-dim mt-1">
                  <Rich>{gs.status.meaning[s.key]}</Rich>
                </p>
              </div>
            </div>
          ))}
        </div>
      </Step>

      {/* Step 7 — Logs */}
      <Step word={gs.stepWord} n={7} id="logs" title={gs.stepTitles.logs}>
        <P>{gs.logs.intro}</P>
        <div className="grid grid-cols-2 gap-4 max-[620px]:grid-cols-1">
          <div className="border border-line rounded-card bg-raise p-4 flex flex-col gap-2">
            <span className="font-ui font-semibold text-[13.5px] text-fg">{gs.logs.browserTitle}</span>
            <p className="text-[13px] leading-relaxed text-dim">
              <Rich>{gs.logs.browser}</Rich>
            </p>
          </div>
          <div className="border border-line rounded-card bg-raise p-4 flex flex-col gap-2">
            <span className="font-ui font-semibold text-[13.5px] text-fg">{gs.logs.apiTitle}</span>
            <p className="text-[13px] leading-relaxed text-dim">
              <Rich>{gs.logs.api}</Rich>
            </p>
          </div>
        </div>
        <CodeBlock caption={gs.logs.terminalCaption} code={CURL_LOGS} />
        <Callout tone="note" title={gs.logs.calloutTitle}>
          <Rich>{gs.logs.callout}</Rich>
        </Callout>
      </Step>

      {/* Step 8 — Lifecycle */}
      <Step word={gs.stepWord} n={8} id="lifecycle" title={gs.stepTitles.lifecycle}>
        <div className="grid grid-cols-3 gap-4 max-[720px]:grid-cols-1">
          {gs.lifecycle.map((card) => (
            <div key={card.t} className="border border-line rounded-card bg-raise p-4 flex flex-col gap-2">
              <span className="font-ui font-semibold text-[13.5px] text-fg">{card.t}</span>
              <p className="text-[13px] leading-relaxed text-dim">
                <Rich>{card.d}</Rich>
              </p>
            </div>
          ))}
        </div>
      </Step>

      {/* Rules of thumb */}
      <section id="rules" className="scroll-mt-[80px]">
        <Eyebrow>{gs.rulesEyebrow}</Eyebrow>
        <h2 className="mt-4 font-display font-semibold text-[20px] tracking-[-0.01em] text-fg">
          {gs.rulesTitle}
        </h2>
        <ul className="mt-5 flex flex-col border border-line rounded-panel overflow-hidden bg-line gap-px">
          {gs.rules.map((r, i) => (
            <li key={i} className="bg-raise p-4 flex items-start gap-3">
              <span className="font-mono text-[11px] text-amber tabular-nums flex-none mt-0.5">
                {String(i + 1).padStart(2, '0')}
              </span>
              <span className="text-[13.5px] leading-relaxed text-dim">
                <Rich>{r}</Rich>
              </span>
            </li>
          ))}
        </ul>
        <p className="mt-6 text-[13px] text-ghost font-mono">
          {gs.fullRefPrefix}{' '}
          <a
            href={MANUAL_URL}
            target="_blank"
            rel="noreferrer"
            className="text-faint hover:text-fg underline decoration-line-2"
          >
            {gs.fullRefLink}
          </a>
        </p>
      </section>
    </div>
  );
}
