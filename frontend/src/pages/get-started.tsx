import React, { useEffect } from 'react';
import { cn } from '@/lib/utils';
import { Eyebrow } from '@/components/layout/eyebrow';
import { CodeBlock } from '@/components/content/code-block';
import { Callout } from '@/components/content/callout';

const REPO_URL = 'https://github.com/ChronoAIProject/fkst-hosted';

const STEPS: { id: string; title: string }[] = [
  { id: 'install', title: 'Install the GitHub App' },
  { id: 'start', title: 'Start a session — open a trigger issue' },
  { id: 'parameters', title: 'Trigger parameters & arguments' },
  { id: 'packages', title: 'Package references' },
  { id: 'queue', title: 'Queue work — open work-label issues' },
  { id: 'status', title: 'Watch the status it writes back' },
  { id: 'logs', title: 'Download a session’s logs' },
  { id: 'lifecycle', title: 'Start, stop & idle' },
];

const TRIGGER_FIELDS: { field: string; required: boolean; rule: React.ReactNode }[] = [
  {
    field: '### Session Name',
    required: true,
    rule: 'Exactly one non-empty line. DNS-label-ish (lowercase letters, digits and dashes) so it composes cleanly into Kubernetes object names.',
  },
  {
    field: '### Packages',
    required: true,
    rule: 'One or more lines, each a fully-qualified package reference owner/repo@ref:path (see the grammar below).',
  },
  {
    field: '### Work Label',
    required: true,
    rule: 'Exactly one non-empty line — a valid GitHub label, ≤ 50 characters, with no comma.',
  },
  {
    field: '### Environment',
    required: false,
    rule: 'One pre-provisioned environment name to inject, or blank for none. It only selects a profile provisioned out-of-band — never put secret values here.',
  },
  {
    field: '### Auto-merge',
    required: false,
    rule: 'true / yes / on / enabled / 1 (case-insensitive) turns it on: the App bot’s PRs are auto-merged into the default branch and the linked work issue auto-closed. Anything else is off.',
  },
  {
    field: '### Log Access Allowlist',
    required: false,
    rule: 'Extra GitHub logins or numeric ids — beyond the author and global admins — allowed to download this session’s logs. Whitespace/comma/newline separated; a leading @ is stripped. Frozen at registration.',
  },
];

type Tone = 'green' | 'neutral' | 'amber' | 'red';

const SIGNALS: { name: string; kind: string; where: string; tone: Tone; meaning: string }[] = [
  {
    name: 'session … registered',
    kind: 'comment',
    where: 'trigger issue',
    tone: 'green',
    meaning: 'Session accepted. The comment carries the 📥 Logs URL and a hidden config-hash marker.',
  },
  {
    name: 'pick-up',
    kind: 'comment',
    where: 'work issue',
    tone: 'neutral',
    meaning: 'The session claimed this work item.',
  },
  {
    name: 'PR by the App bot',
    kind: 'pull request',
    where: 'repo',
    tone: 'neutral',
    meaning: 'The session’s output for a work item.',
  },
  {
    name: 'fkst-degraded',
    kind: 'label',
    where: 'trigger issue',
    tone: 'amber',
    meaning: 'The pod looks unhealthy (crash/restart or a recurring error). Cleared when it reads healthy again.',
  },
  {
    name: 'fkst-session-retired',
    kind: 'label',
    where: 'open work issues',
    tone: 'red',
    meaning: 'The trigger was closed → the session retired; the item is no longer worked.',
  },
  {
    name: 'fkst-substrate-invalid',
    kind: 'label',
    where: 'trigger issue',
    tone: 'red',
    meaning: 'The body failed to parse, or a package is unreachable. Fix it and the flag clears next sweep.',
  },
  {
    name: 'fkst-config-rejected',
    kind: 'label',
    where: 'trigger issue',
    tone: 'red',
    meaning: 'You edited the config of an already-registered session (config is frozen).',
  },
];

const DOT_TONE: Record<Tone, string> = {
  green: 'bg-green',
  neutral: 'bg-ghost',
  amber: 'bg-amber',
  red: 'bg-red',
};

const RULES_OF_THUMB: string[] = [
  'One Work Label per open trigger, per repo. Two open triggers sharing a label spawn competing pods over the same queue — double-claims and duplicate PRs.',
  'Wave the backlog by dependency. Land foundational work issues, merge them, then open the issues that build on them. Dependency ordering — not wording — is the usual failure mode.',
  'One feature per work issue, named in the title, with exact files and checkable acceptance criteria.',
  'Never put secrets, tokens, or env values in an issue. Use ### Environment to select a pre-provisioned profile; values are supplied out of band.',
  'Give it a sweep. Actions reconcile on a poll — expect seconds, and re-check the issue’s comments and labels rather than expecting an instant effect.',
];

const TRIGGER_EXAMPLE = `### Session Name
sitebuilder

### Packages
ChronoAIProject/fkst-packages@dev:packages/github-devloop
ChronoAIProject/fkst-packages@dev:packages/github-devloop-pr
ChronoAIProject/fkst-packages@dev:packages/github-devloop-ops
ChronoAIProject/fkst-packages@dev:packages/consensus

### Work Label
site-build

### Auto-merge
true`;

const GH_CREATE = `gh issue create \\
  --repo <owner>/<repo> \\
  --title "[session] sitebuilder" \\
  --body-file body.md \\
  --label fkst-substrate-trigger`;

const CURL_LOGS = `curl -L \\
  -H "Authorization: Bearer $GITHUB_TOKEN" \\
  https://<host>/api/v1/logs/<session_id> \\
  -o logs.tar.gz`;

function Step({
  n,
  id,
  title,
  children,
}: {
  n: number;
  id: string;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section id={id} className="scroll-mt-[80px]">
      <Eyebrow>Step {n}</Eyebrow>
      <h2 className="mt-4 font-display font-semibold text-[22px] leading-[1.2] tracking-[-0.01em] text-fg">
        {title}
      </h2>
      <div className="mt-5 flex flex-col gap-5">{children}</div>
    </section>
  );
}

function P({ children }: { children: React.ReactNode }) {
  return <p className="text-[14px] leading-relaxed text-dim max-w-[68ch]">{children}</p>;
}

function Mono({ children }: { children: React.ReactNode }) {
  return (
    <code className="font-mono text-[12.5px] text-fg bg-raise-2 rounded-chip px-1 py-0.5">
      {children}
    </code>
  );
}

export function GetStarted() {
  useEffect(() => {
    document.title = 'FKST — Get Started';
  }, []);

  return (
    <div className="flex flex-col gap-16 max-w-[880px]">
      {/* Header */}
      <header>
        <Eyebrow>Get Started</Eyebrow>
        <h1 className="mt-5 font-display font-bold text-[clamp(28px,4vw,40px)] leading-[1.1] tracking-[-0.02em] text-fg">
          Drive fkst-hosted with GitHub issues
        </h1>
        <p className="mt-5 text-[15px] leading-relaxed text-dim max-w-[68ch]">
          You control everything through GitHub issues — there is no dashboard and no REST API you
          drive by hand. Install the App once, open a trigger issue to start a session, then queue
          work as more issues. Every action reconciles on a poll, so expect effects within a sweep
          (seconds), not instantly.
        </p>

        {/* Step index */}
        <nav className="mt-8 border border-line rounded-panel bg-raise overflow-hidden">
          <ol className="grid grid-cols-2 max-[640px]:grid-cols-1 gap-px bg-line">
            {STEPS.map((s, i) => (
              <li key={s.id} className="bg-raise">
                <a
                  href={`#${s.id}`}
                  className="flex items-baseline gap-3 px-4 py-3 no-underline group hover:bg-raise-2 transition-colors"
                >
                  <span className="font-mono text-[11px] text-ghost tabular-nums flex-none">
                    {String(i + 1).padStart(2, '0')}
                  </span>
                  <span className="text-[13.5px] text-dim group-hover:text-fg transition-colors">
                    {s.title}
                  </span>
                </a>
              </li>
            ))}
          </ol>
        </nav>
      </header>

      {/* Step 1 — Install */}
      <Step n={1} id="install" title="Install the GitHub App">
        <P>
          Install ChronoAI’s <span className="text-fg">fkst-hosted</span> GitHub App on the
          repositories you want sessions to run in. The App is what opens pull requests, writes
          status back to your issues, and reconciles declared state (your open trigger issues)
          toward reality (one pod per live session).
        </P>
        <Callout tone="warn" title="Access it needs">
          The App must be installed on the repo <em>and</em> able to reach every package reference —
          each must be public, or in a repo the App can read. An unreachable ref makes the reconciler
          flag the trigger <Mono>fkst-substrate-invalid</Mono> until you fix it.
        </Callout>
      </Step>

      {/* Step 2 — Start a session */}
      <Step n={2} id="start" title="Start a session — open a trigger issue">
        <P>
          Open a GitHub issue <span className="text-fg">labeled <Mono>fkst-substrate-trigger</Mono></span>{' '}
          whose body has the <Mono>###</Mono> sections below (matched by exact heading; a duplicate
          heading makes the issue invalid). Any intro text before the first heading is ignored. One
          trigger issue creates exactly one session.
        </P>
        <CodeBlock caption="body.md — trigger issue body" code={TRIGGER_EXAMPLE} />
        <P>Create it from the CLI:</P>
        <CodeBlock caption="terminal" code={GH_CREATE} />
        <Callout tone="warn" title="If the body is wrong">
          A malformed body or an unreachable package makes the reconciler label the trigger{' '}
          <Mono>fkst-substrate-invalid</Mono> and comment with the fix. Correct the body and the flag
          clears on the next sweep.
        </Callout>
      </Step>

      {/* Step 3 — Parameters */}
      <Step n={3} id="parameters" title="Trigger parameters & arguments">
        <P>
          Each <Mono>###</Mono> section of the trigger body is one parameter. Three are required; the
          rest are optional.
        </P>
        <div className="border border-line rounded-panel overflow-hidden flex flex-col gap-px bg-line">
          {TRIGGER_FIELDS.map((f) => (
            <div
              key={f.field}
              className="bg-raise p-4 grid grid-cols-[minmax(180px,220px)_1fr] gap-4 max-[620px]:grid-cols-1 max-[620px]:gap-2"
            >
              <div className="flex flex-col gap-2 min-w-0">
                <code className="font-mono text-[12.5px] text-fg break-words">{f.field}</code>
                <span
                  className={cn(
                    'font-mono text-[10px] uppercase tracking-[0.1em] font-semibold w-fit px-1.5 py-0.5 rounded-chip border',
                    f.required
                      ? 'text-amber border-[color-mix(in_oklab,var(--amber)_40%,var(--line))]'
                      : 'text-ghost border-line-2'
                  )}
                >
                  {f.required ? 'required' : 'optional'}
                </span>
              </div>
              <p className="text-[13px] leading-relaxed text-dim min-w-0">{f.rule}</p>
            </div>
          ))}
        </div>
        <Callout tone="note" title="Config is immutable">
          Once a session has registered, its config (packages, work label, environment, auto-merge,
          log allow-list) is frozen. Editing the trigger body does <em>not</em> relaunch it — the
          control plane posts a one-time <Mono>fkst-config-rejected</Mono> comment. To change config,
          close the trigger and open a new one.
        </Callout>
      </Step>

      {/* Step 4 — Package refs */}
      <Step n={4} id="packages" title="Package references — owner/repo@ref:path">
        <P>
          Each line under <Mono>### Packages</Mono> is one reference. It’s split greedily on the
          first <Mono>@</Mono> (into <Mono>owner/repo</Mono> and <Mono>ref:path</Mono>), then on the
          first <Mono>:</Mono> (into <Mono>ref</Mono> and <Mono>path</Mono>).
        </P>
        <div className="grid grid-cols-3 gap-px bg-line border border-line rounded-panel overflow-hidden max-[620px]:grid-cols-1">
          {[
            {
              part: 'owner/repo',
              rule: 'Matches [A-Za-z0-9_.-]+, with exactly one slash between owner and repo.',
            },
            {
              part: 'ref',
              rule: 'A branch, tag or SHA — [A-Za-z0-9_./-]+, with no “..” segment.',
            },
            {
              part: 'path',
              rule: 'Repo-relative — [A-Za-z0-9_./-]+, never absolute and with no “..” segment.',
            },
          ].map((g) => (
            <div key={g.part} className="bg-raise p-4 flex flex-col gap-2">
              <code className="font-mono text-[12.5px] text-fg">{g.part}</code>
              <span className="text-[12.5px] leading-relaxed text-dim">{g.rule}</span>
            </div>
          ))}
        </div>
        <CodeBlock
          caption="a single package reference"
          code="ChronoAIProject/fkst-packages@dev:packages/github-devloop"
        />
      </Step>

      {/* Step 5 — Queue work */}
      <Step n={5} id="queue" title="Queue work — open work-label issues">
        <P>
          Open one issue <span className="text-fg">per task</span>, labeled with the session’s Work
          Label. Give each a clear title, the exact files to change, real acceptance criteria, and
          enough spec to be worked in isolation — the agent sees that one issue plus the repo, not
          the sibling backlog. The session picks them up, opens a pull request per issue, and (when
          Auto-merge is on) merges and closes them.
        </P>
        <Callout tone="tip" title="Keep the queue healthy">
          An open work issue keeps the pod alive; merge or close finished work to let a session idle
          down. And never give two open triggers in one repo the same work label.
        </Callout>
      </Step>

      {/* Step 6 — Status */}
      <Step n={6} id="status" title="Watch the status it writes back">
        <P>
          The control plane reports progress on the same issues, as comments and labels. You apply
          only <Mono>fkst-substrate-trigger</Mono> and your Work Label — every other{' '}
          <Mono>fkst-*</Mono> label below is managed for you.
        </P>
        <div className="border border-line rounded-panel overflow-hidden flex flex-col gap-px bg-line">
          {SIGNALS.map((s) => (
            <div
              key={s.name}
              className="bg-raise p-4 grid grid-cols-[minmax(0,1.1fr)_minmax(0,1.4fr)] gap-4 items-start max-[620px]:grid-cols-1 max-[620px]:gap-2"
            >
              <div className="flex items-center gap-2.5 min-w-0">
                <span
                  className={cn('w-2 h-2 rounded-full flex-none', DOT_TONE[s.tone])}
                  aria-hidden="true"
                />
                <code className="font-mono text-[12.5px] text-fg truncate min-w-0">{s.name}</code>
                <span className="font-mono text-[10px] uppercase tracking-[0.08em] text-ghost border border-line-2 rounded-chip px-1.5 py-0.5 flex-none">
                  {s.kind}
                </span>
              </div>
              <div className="min-w-0">
                <span className="font-mono text-[11px] text-ghost">on {s.where}</span>
                <p className="text-[13px] leading-relaxed text-dim mt-1">{s.meaning}</p>
              </div>
            </div>
          ))}
        </div>
      </Step>

      {/* Step 7 — Logs */}
      <Step n={7} id="logs" title="Download a session’s logs">
        <P>
          Every session auto-streams its redacted logs to storage. The 📥 Logs URL in the
          registration comment is <Mono>/api/v1/logs/{'{session_id}'}</Mono>. Access is
          identity-gated and deny-by-default — authorized only if you are the trigger author, on the{' '}
          <Mono>### Log Access Allowlist</Mono>, or a global admin. There are two ways in:
        </P>
        <div className="grid grid-cols-2 gap-4 max-[620px]:grid-cols-1">
          <div className="border border-line rounded-card bg-raise p-4 flex flex-col gap-2">
            <span className="font-ui font-semibold text-[13.5px] text-fg">Browser</span>
            <p className="text-[13px] leading-relaxed text-dim">
              Open the URL. It redirects through GitHub login, then the redacted{' '}
              <Mono>.tar.gz</Mono> downloads. No storage URL is ever exposed — the control plane
              streams the bytes.
            </p>
          </div>
          <div className="border border-line rounded-card bg-raise p-4 flex flex-col gap-2">
            <span className="font-ui font-semibold text-[13.5px] text-fg">Agent / API</span>
            <p className="text-[13px] leading-relaxed text-dim">
              Send a bearer GitHub token; it’s traded for your identity and the redacted{' '}
              <Mono>.tar.gz</Mono> streams back.
            </p>
          </div>
        </div>
        <CodeBlock caption="terminal" code={CURL_LOGS} />
        <Callout tone="note" title="What you get">
          Logs are the latest flush — refreshed roughly every 20 s / 256 KB and on pod exit — and are
          redacted (secrets masked). Safe to share with an authorized user, but treat them as
          session-sensitive.
        </Callout>
      </Step>

      {/* Step 8 — Lifecycle */}
      <Step n={8} id="lifecycle" title="Start, stop & idle">
        <div className="grid grid-cols-3 gap-4 max-[720px]:grid-cols-1">
          {[
            {
              t: 'Permanent stop',
              d: 'Close the trigger issue. The session retires, the pod is cleaned up, and it never revives — a closed trigger is never re-registered. Open work issues get fkst-session-retired.',
            },
            {
              t: 'Idle (auto-revive)',
              d: 'Trigger open, no open work → the pod is killed to save resources, but the session respawns the moment a matching work issue appears. No new trigger needed.',
            },
            {
              t: 'Keep it running',
              d: 'An open work issue keeps the pod alive. To pause, close or merge all work; to resume, open a work issue.',
            },
          ].map((c) => (
            <div key={c.t} className="border border-line rounded-card bg-raise p-4 flex flex-col gap-2">
              <span className="font-ui font-semibold text-[13.5px] text-fg">{c.t}</span>
              <p className="text-[13px] leading-relaxed text-dim">{c.d}</p>
            </div>
          ))}
        </div>
      </Step>

      {/* Rules of thumb */}
      <section id="rules" className="scroll-mt-[80px]">
        <Eyebrow>Rules of thumb</Eyebrow>
        <h2 className="mt-4 font-display font-semibold text-[20px] tracking-[-0.01em] text-fg">
          Learned the hard way
        </h2>
        <ul className="mt-5 flex flex-col border border-line rounded-panel overflow-hidden bg-line gap-px">
          {RULES_OF_THUMB.map((r, i) => (
            <li key={i} className="bg-raise p-4 flex items-start gap-3">
              <span className="font-mono text-[11px] text-amber tabular-nums flex-none mt-0.5">
                {String(i + 1).padStart(2, '0')}
              </span>
              <span className="text-[13.5px] leading-relaxed text-dim">{r}</span>
            </li>
          ))}
        </ul>
        <p className="mt-6 text-[13px] text-ghost font-mono">
          Full reference:{' '}
          <a
            href={`${REPO_URL}/blob/main/skills/fkst-control-plane-manual/SKILL.md`}
            target="_blank"
            rel="noreferrer"
            className="text-faint hover:text-fg underline decoration-line-2"
          >
            the operator manual ↗
          </a>
        </p>
      </section>
    </div>
  );
}
