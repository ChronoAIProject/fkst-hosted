import { useEffect } from 'react';
import { Link } from 'react-router-dom';
import { Eyebrow } from '@/components/layout/eyebrow';
import { FkstMark } from '@/components/brand/fkst-mark';

const REPO_URL = 'https://github.com/ChronoAIProject/fkst-hosted';

const MENTAL_MODEL: { term: string; is: string; control: string }[] = [
  {
    term: 'Session',
    is: 'One long-lived coding agent — a single Kubernetes pod.',
    control: 'You control it by opening or closing a trigger issue.',
  },
  {
    term: 'Trigger issue',
    is: "The session's declaration: its name, packages, and work label.",
    control: 'A GitHub issue labeled fkst-substrate-trigger.',
  },
  {
    term: 'Work item',
    is: 'One task for the session to pick up — it becomes a pull request.',
    control: "Any issue carrying the session's work label.",
  },
];

const FEATURES: { title: string; body: string }[] = [
  {
    title: 'Managed engine on Kubernetes',
    body: 'One pod per live session, auto-provisioned and auto-cleaned. No clusters to run, patch, or scale yourself.',
  },
  {
    title: 'GitHub-native control',
    body: 'Start, queue, watch, and stop sessions entirely through issues. Progress comes back as comments and labels on those same issues.',
  },
  {
    title: 'A pull request per task',
    body: 'Each work item becomes its own isolated PR. Turn on auto-merge and finished work lands on your default branch and closes itself.',
  },
  {
    title: 'Idle to zero, auto-revive',
    body: 'With no open work a session idles and its pod is reclaimed. Open a new work issue and it respawns on its own — same session, no re-setup.',
  },
  {
    title: 'Redacted logs, identity-gated',
    body: 'Every session streams redacted logs to storage. Download them from an identity-gated endpoint — trigger author, allow-list, or admin only.',
  },
  {
    title: 'Safe by construction',
    body: 'Registered config is frozen and secrets never live in issues — you select a pre-provisioned environment profile, and values are supplied out of band.',
  },
];

const FLOW: { label: string; sub: string }[] = [
  { label: 'Trigger issue', sub: 'declare a session' },
  { label: 'Session', sub: 'one K8s pod' },
  { label: 'Work issues', sub: 'the queue' },
  { label: 'PR per task', sub: 'isolated change' },
  { label: 'Merge', sub: 'optional auto-merge' },
];

export function Introduction() {
  useEffect(() => {
    document.title = 'FKST — Autonomous coding sessions, hosted';
  }, []);

  return (
    <div className="flex flex-col gap-20 max-[600px]:gap-16">
      {/* Hero */}
      <section className="pt-4">
        <Eyebrow>ChronoAI · fkst-hosted</Eyebrow>
        <h1 className="mt-5 font-display font-bold text-[clamp(30px,5vw,52px)] leading-[1.05] tracking-[-0.02em] text-fg max-w-[16ch]">
          Autonomous coding sessions, driven entirely by GitHub issues.
        </h1>
        <p className="mt-6 text-[16px] leading-relaxed text-dim max-w-[62ch]">
          <FkstMark className="text-[16px] text-fg" /> hosted is ChronoAI's managed cloud for the
          fkst engine. Open an issue to start a long-lived coding agent, queue tasks as more issues,
          and it opens a pull request per task. No infrastructure to operate, no dashboard to learn.
        </p>
        <div className="mt-8 flex items-center gap-3 flex-wrap">
          <Link
            to="/get-started"
            className="font-ui font-semibold text-[13.5px] bg-amber text-amber-ink rounded-control px-5 py-2.5 no-underline transition-colors hover:brightness-[1.06]"
          >
            Get started →
          </Link>
          <a
            href={`${REPO_URL}/blob/main/skills/fkst-control-plane-manual/SKILL.md`}
            target="_blank"
            rel="noreferrer"
            className="font-ui font-medium text-[13.5px] text-dim border border-line-2 bg-raise rounded-control px-5 py-2.5 no-underline transition-colors hover:text-fg hover:border-faint"
          >
            Read the operator manual ↗
          </a>
        </div>
      </section>

      {/* What is FKST */}
      <section>
        <Eyebrow>What is FKST</Eyebrow>
        <div className="mt-5 grid grid-cols-[1.1fr_0.9fr] gap-x-12 gap-y-6 max-[820px]:grid-cols-1">
          <h2 className="font-display font-semibold text-[24px] leading-[1.2] tracking-[-0.01em] text-fg">
            An engine for autonomous, package-driven coding agents — run for you as a service.
          </h2>
          <div className="flex flex-col gap-4 text-[14px] leading-relaxed text-dim">
            <p>
              <span className="text-fg font-medium">fkst</span> is an engine (fkst-substrate) that
              runs autonomous coding agents. Its behavior comes from{' '}
              <span className="text-fg">packages</span> — small bundles the engine loads and runs
              against a GitHub repository.
            </p>
            <p>
              <span className="text-fg font-medium">fkst-hosted</span> runs that engine for you as
              ChronoAI's cloud service. There's no dashboard and no REST API you drive by hand — the
              entire control surface is GitHub issues. You start a session by opening an issue, queue
              work with more issues, and stop it by closing them.
            </p>
          </div>
        </div>

        {/* One-line thesis */}
        <div className="mt-10 border border-line border-l-2 border-l-amber rounded-card bg-[color-mix(in_oklab,var(--raise)_55%,transparent)] px-5 py-4">
          <p className="text-[15px] leading-relaxed text-fg">
            One trigger issue <span className="text-ghost">=</span> one session. Open work-label
            issues <span className="text-ghost">=</span> the queue that session works, each as its
            own pull request.
          </p>
        </div>
      </section>

      {/* Mental model */}
      <section>
        <Eyebrow>The mental model</Eyebrow>
        <div className="mt-5 grid grid-cols-3 gap-px bg-line border border-line rounded-panel overflow-hidden max-[720px]:grid-cols-1">
          {MENTAL_MODEL.map((m) => (
            <div key={m.term} className="bg-raise p-5 flex flex-col gap-2.5">
              <span className="font-display font-semibold text-[15px] tracking-[0.01em] text-fg">
                {m.term}
              </span>
              <span className="text-[13px] leading-relaxed text-dim">{m.is}</span>
              <span className="font-mono text-[11.5px] leading-relaxed text-ghost mt-auto pt-1">
                {m.control}
              </span>
            </div>
          ))}
        </div>
      </section>

      {/* What the hosted service provides */}
      <section>
        <Eyebrow>What the hosted service provides</Eyebrow>
        <h2 className="mt-5 font-display font-semibold text-[24px] leading-[1.2] tracking-[-0.01em] text-fg max-w-[24ch]">
          A managed home for the engine — you bring intent, it brings the infrastructure.
        </h2>
        <div className="mt-8 grid grid-cols-3 gap-4 max-[900px]:grid-cols-2 max-[600px]:grid-cols-1">
          {FEATURES.map((f) => (
            <div
              key={f.title}
              className="border border-line rounded-card bg-raise p-5 flex flex-col gap-2.5 transition-colors hover:border-line-2"
            >
              <span className="w-1.5 h-1.5 rounded-full bg-amber" aria-hidden="true" />
              <h3 className="font-ui font-semibold text-[14.5px] text-fg leading-snug">
                {f.title}
              </h3>
              <p className="text-[13px] leading-relaxed text-dim">{f.body}</p>
            </div>
          ))}
        </div>
      </section>

      {/* Flow */}
      <section>
        <Eyebrow>How it flows</Eyebrow>
        <div className="mt-6 flex items-stretch gap-0 max-[760px]:flex-col">
          {FLOW.map((step, idx) => (
            <div key={step.label} className="flex items-stretch flex-1 min-w-0 max-[760px]:flex-none">
              <div className="flex-1 min-w-0 border border-line rounded-card bg-raise px-4 py-4 flex flex-col justify-center gap-1">
                <span className="font-mono text-[10.5px] text-ghost tabular-nums">
                  {String(idx + 1).padStart(2, '0')}
                </span>
                <span className="font-display font-semibold text-[14px] text-fg leading-tight">
                  {step.label}
                </span>
                <span className="font-mono text-[11px] text-ghost">{step.sub}</span>
              </div>
              {idx < FLOW.length - 1 && (
                <div
                  className="flex items-center justify-center px-2 text-ghost flex-none max-[760px]:h-4 max-[760px]:rotate-90 max-[760px]:self-center"
                  aria-hidden="true"
                >
                  →
                </div>
              )}
            </div>
          ))}
        </div>
      </section>

      {/* CTA band */}
      <section className="border border-line rounded-panel bg-[color-mix(in_oklab,var(--raise)_60%,transparent)] px-8 py-10 max-[600px]:px-5 max-[600px]:py-8 flex items-center justify-between gap-6 flex-wrap">
        <div>
          <h2 className="font-display font-semibold text-[22px] tracking-[-0.01em] text-fg">
            Open an issue. Get a pull request.
          </h2>
          <p className="mt-2 text-[14px] text-dim max-w-[52ch]">
            Install the GitHub App, open a trigger issue, and queue your first task. It reconciles
            within a sweep — seconds, not instant.
          </p>
        </div>
        <Link
          to="/get-started"
          className="font-ui font-semibold text-[13.5px] bg-amber text-amber-ink rounded-control px-5 py-2.5 no-underline transition-colors hover:brightness-[1.06] flex-none"
        >
          Get started →
        </Link>
      </section>
    </div>
  );
}
