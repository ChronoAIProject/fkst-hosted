import type { PagesSlice } from '../slices';

// The long-form marketing / get-started pages (`intro` + `gs`). Kept together
// because they share tone and are edited as a pair; composed into the top level.
export const pages: PagesSlice = {
  intro: {
    metaTitle: 'FKST — Autonomous coding sessions, hosted',
    eyebrow: 'ChronoAI',
    heroTitleTop: 'Open an issue.',
    heroTitleAccent: 'Get a pull request.',
    heroLede:
      'FKST runs long-lived coding agents driven entirely by GitHub issues. No infrastructure, nothing to learn.',
    ctaStart: 'Get started',
    ctaManual: 'Operator manual →',
    pipeTrigger: 'trigger issue',
    pipeSession: 'live session',
    pipeWork: 'a PR per task',
  },
  gs: {
    metaTitle: 'FKST — Get Started',
    eyebrow: 'Get Started',
    title: 'Drive fkst-hosted with GitHub issues',
    lede: 'You control everything through GitHub issues — there is no dashboard and no REST API you drive by hand. Install the App once, open a trigger issue to start a session, then queue work as more issues. Every action reconciles on a poll, so expect effects within a sweep (seconds), not instantly.',
    stepWord: 'Step',
    stepTitles: {
      install: 'Install the GitHub App',
      start: 'Start a session — open a trigger issue',
      parameters: 'Trigger parameters & arguments',
      packages: 'Package references',
      queue: 'Queue work — open work-label issues',
      status: 'Watch the status it writes back',
      logs: 'Download a session’s logs',
      lifecycle: 'Start, stop & idle',
    },
    requiredLabel: 'required',
    optionalLabel: 'optional',
    install: {
      body: 'Install ChronoAI’s **fkst-hosted** GitHub App on the repositories you want sessions to run in. The App is what opens pull requests, writes status back to your issues, and reconciles declared state (your open trigger issues) toward reality (one pod per live session).',
      calloutTitle: 'Access it needs',
      callout:
        'The App must be installed on the repo *and* able to reach every package reference — each must be public, or in a repo the App can read. An unreachable ref makes the reconciler flag the trigger `fkst-substrate-invalid` until you fix it.',
    },
    start: {
      body: 'Open a GitHub issue labeled `fkst-substrate-trigger` whose body has the `###` sections below (matched by exact heading; a duplicate heading makes the issue invalid). Any intro text before the first heading is ignored. One trigger issue creates exactly one session.',
      exampleCaption: 'body.md — trigger issue body',
      createIntro: 'Create it from the CLI:',
      terminalCaption: 'terminal',
      calloutTitle: 'If the body is wrong',
      callout:
        'A malformed body or an unreachable package makes the reconciler label the trigger `fkst-substrate-invalid` and comment with the fix. Correct the body and the flag clears on the next sweep.',
    },
    parameters: {
      intro:
        'Each `###` section of the trigger body is one parameter. Three are required; the rest are optional.',
      fieldRules: {
        sessionName:
          'Exactly one non-empty line. DNS-label-ish (lowercase letters, digits and dashes) so it composes cleanly into Kubernetes object names.',
        packages:
          'One or more lines, each a fully-qualified package reference `owner/repo@ref:path` (see the grammar below).',
        workLabel:
          'Exactly one non-empty line — a valid GitHub label, ≤ 50 characters, with no comma.',
        environment:
          'One pre-provisioned environment name to inject, or blank for none. It only selects a profile provisioned out of band — never put secret values here.',
        autoMerge:
          '`true` / `yes` / `on` / `enabled` / `1` (case-insensitive) turns it on: the App bot’s PRs are auto-merged into the default branch and the linked work issue auto-closed. Anything else is off.',
        logAllowlist:
          'Extra GitHub logins or numeric ids — beyond the author and global admins — allowed to download this session’s logs. Whitespace/comma/newline separated; a leading `@` is stripped. Frozen at registration.',
      },
      calloutTitle: 'Config is immutable',
      callout:
        'Once a session has registered, its config (packages, work label, environment, auto-merge, log allow-list) is frozen. Editing the trigger body does *not* relaunch it — the control plane posts a one-time `fkst-config-rejected` comment. To change config, close the trigger and open a new one.',
    },
    packages: {
      intro:
        'Each line under `### Packages` is one reference. It’s split greedily on the first `@` (into `owner/repo` and `ref:path`), then on the first `:` (into `ref` and `path`).',
      grammar: {
        ownerRepo: 'Matches [A-Za-z0-9_.-]+, with exactly one slash between owner and repo.',
        ref: 'A branch, tag or SHA — [A-Za-z0-9_./-]+, with no “..” segment.',
        path: 'Repo-relative — [A-Za-z0-9_./-]+, never absolute and with no “..” segment.',
      },
      exampleCaption: 'a single package reference',
    },
    queue: {
      body: 'Open one issue **per task**, labeled with the session’s Work Label. Give each a clear title, the exact files to change, real acceptance criteria, and enough spec to be worked in isolation — the agent sees that one issue plus the repo, not the sibling backlog. The session picks them up, opens a pull request per issue, and (when Auto-merge is on) merges and closes them.',
      calloutTitle: 'Keep the queue healthy',
      callout:
        'An open work issue keeps the pod alive; merge or close finished work to let a session idle down. And never give two open triggers in one repo the same work label.',
    },
    status: {
      intro:
        'The control plane reports progress on the same issues, as comments and labels. You apply only `fkst-substrate-trigger` and your Work Label — every other `fkst-*` label below is managed for you.',
      onWord: 'on',
      kind: {
        registered: 'comment',
        pickup: 'comment',
        pr: 'pull request',
        degraded: 'label',
        retired: 'label',
        invalid: 'label',
        configRejected: 'label',
      },
      where: {
        registered: 'trigger issue',
        pickup: 'work issue',
        pr: 'repo',
        degraded: 'trigger issue',
        retired: 'open work issues',
        invalid: 'trigger issue',
        configRejected: 'trigger issue',
      },
      meaning: {
        registered:
          'Session accepted. The comment carries the 📥 Logs URL and a hidden config-hash marker.',
        pickup: 'The session claimed this work item.',
        pr: 'The session’s output for a work item.',
        degraded:
          'The pod looks unhealthy (crash/restart or a recurring error). Cleared when it reads healthy again.',
        retired: 'The trigger was closed → the session retired; the item is no longer worked.',
        invalid:
          'The body failed to parse, or a package is unreachable. Fix it and the flag clears next sweep.',
        configRejected: 'You edited the config of an already-registered session (config is frozen).',
      },
    },
    logs: {
      intro:
        'Every session auto-streams its redacted logs to storage. The 📥 Logs URL in the registration comment is `/api/v1/logs/{session_id}`. Access is identity-gated and deny-by-default — authorized only if you are the trigger author, on the `### FKST Contributors` list, or a global admin. There are two ways in:',
      browserTitle: 'Browser',
      browser:
        'Open the URL. It redirects through GitHub login, then the redacted `.tar.gz` downloads. No storage URL is ever exposed — the control plane streams the bytes.',
      apiTitle: 'Agent / API',
      api: 'Send a bearer GitHub token; it’s traded for your identity and the redacted `.tar.gz` streams back.',
      terminalCaption: 'terminal',
      calloutTitle: 'What you get',
      callout:
        'Logs are the latest flush — refreshed roughly every 20 s / 256 KB and on pod exit — and are redacted (secrets masked). Safe to share with an authorized user, but treat them as session-sensitive.',
    },
    lifecycle: [
      {
        t: 'Permanent stop',
        d: 'Close the trigger issue. The session retires, the pod is cleaned up, and it never revives — a closed trigger is never re-registered. Open work issues get `fkst-session-retired`.',
      },
      {
        t: 'Idle (auto-revive)',
        d: 'Trigger open, no open work → the pod is killed to save resources, but the session respawns the moment a matching work issue appears. No new trigger needed.',
      },
      {
        t: 'Keep it running',
        d: 'An open work issue keeps the pod alive. To pause, close or merge all work; to resume, open a work issue.',
      },
    ],
    rulesEyebrow: 'Rules of thumb',
    rulesTitle: 'Learned the hard way',
    rules: [
      'One Work Label per open trigger, per repo. Two open triggers sharing a label spawn competing pods over the same queue — double-claims and duplicate PRs.',
      'Wave the backlog by dependency. Land foundational work issues, merge them, then open the issues that build on them. Dependency ordering — not wording — is the usual failure mode.',
      'One feature per work issue, named in the title, with exact files and checkable acceptance criteria.',
      'Never put secrets, tokens, or env values in an issue. Use `### Environment` to select a pre-provisioned profile; values are supplied out of band.',
      'Give it a sweep. Actions reconcile on a poll — expect seconds, and re-check the issue’s comments and labels rather than expecting an instant effect.',
    ],
    fullRefPrefix: 'Full reference:',
    fullRefLink: 'the operator manual ↗',
  },
};
