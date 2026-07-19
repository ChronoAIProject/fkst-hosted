import type { Node, NodeProps } from '@xyflow/react';
import { cn } from '@/lib/utils';
import { useContent } from '@/i18n';
import { Chip } from '@/components/ui/chip';
import { FadeSwap, staggerStyle } from '@/components/ui/motion';
import { accountStatus, repoDetailStatus, repoStatus, sessionActive } from '@/lib/api/derive';
import type { CanvasStatus } from '@/lib/api/derive';
import type { AccountOverview, RepoOverview, RepoSessionsResponse } from '@/lib/api/types';
import { ACCOUNT_NODE, DETAIL_NODE, REPO_NODE } from './layout';

// Custom node payloads. Type aliases (not interfaces) so they satisfy React
// Flow's Record<string, unknown> node-data constraint structurally.
//
// `index` is the node's position within its level. It feeds the mount stagger
// (`anim-row-in` + `--stagger`) so a level's cards unfold as a cascading set
// rather than snapping in together. It is OPTIONAL because the producer
// (`flow.tsx::buildNodes`) is a sibling-owned file: absent it, `staggerStyle`
// degrades to a zero delay and every card fades in on the same frame — still
// animated, just not cascaded. See the cross-item note in the PR.
export type AccountNodeData = {
  account: AccountOverview;
  onOpen: (login: string) => void;
  index?: number;
};
export type RepoNodeData = {
  repo: RepoOverview;
  onOpen: (owner: string, name: string) => void;
  index?: number;
};
export type DetailNodeData = {
  owner: string;
  name: string;
  installed: boolean;
  /** Null while the level-2 fetch is in flight → mini skeleton. */
  sessions: RepoSessionsResponse | null;
  /** True when the level-2 fetch FAILED (distinct from still-loading `null`):
   *  renders a short "could not load" line instead of shimmering forever.
   *  Optional — the sibling-owned producer wires it; absent it reads falsy and
   *  a null payload keeps meaning "loading". */
  sessionsFailed?: boolean;
  index?: number;
};

export type AccountFlowNode = Node<AccountNodeData, 'account'>;
export type RepoFlowNode = Node<RepoNodeData, 'repo'>;
export type DetailFlowNode = Node<DetailNodeData, 'repoDetail'>;
export type CanvasNode = AccountFlowNode | RepoFlowNode | DetailFlowNode;

/** Card chrome for the three status classes: quiet raised (no App), amber-tinted
 *  + resting amber bloom (installed), amber-tinted + blinking glow (active).
 *
 *  Depth is carried by the layered shadow scale + a status-matched glow, and the
 *  hover accent is a box-shadow/border change only — NO transform. That is
 *  deliberate: React Flow owns a positioning `transform` on the outer node
 *  wrapper and the body already runs `anim-row-in` (a transform keyframe with
 *  fill `both`), so a hover translate would be suppressed by the lingering
 *  animation fill anyway. Animating depth via shadow keeps the lift reading
 *  cleanly. The glow keyframes collapse under prefers-reduced-motion; every
 *  state also carries a textual badge so it never rides on motion/color alone. */
function statusCardClasses(status: CanvasStatus): string {
  switch (status) {
    case 'none':
      return 'border-line bg-raise shadow-2 hover:shadow-3 hover:border-line-2';
    case 'installed':
      // shadow-glow = card depth + amber bloom; hover deepens the shadow and
      // brightens the hairline toward the active treatment.
      return 'border-[color-mix(in_oklab,var(--amber)_50%,var(--line))] bg-[color-mix(in_oklab,var(--amber)_8%,var(--raise))] shadow-glow hover:shadow-[var(--shadow-3),var(--glow-amber)] hover:border-[color-mix(in_oklab,var(--amber)_72%,var(--line))]';
    case 'active':
      // anim-node-glow owns box-shadow (a breathing amber pulse), so no static
      // shadow utility here — the pulse is the depth cue for a live node.
      return 'border-[color-mix(in_oklab,var(--amber)_72%,var(--line))] bg-[color-mix(in_oklab,var(--amber)_11%,var(--raise))] anim-node-glow';
  }
}

function StatusBadge({ status, activeCount }: { status: CanvasStatus; activeCount: number }) {
  const cc = useContent().dashboard.canvas;
  if (status === 'active') {
    return (
      <span className="font-mono text-[10.5px] px-1.5 py-0.5 rounded-chip bg-grad-accent text-amber-ink font-semibold shadow-glow-amber">
        {cc.statusActiveCount.replace('{n}', String(activeCount))}
      </span>
    );
  }
  return (
    <span className={cn('font-mono text-[10.5px]', status === 'installed' ? 'text-amber' : 'text-ghost')}>
      {status === 'installed' ? cc.statusInstalled : cc.statusNone}
    </span>
  );
}

/** How many repo dots fit inside an account card before "+N more". */
export const MAX_REPO_DOTS = 18;

function RepoDot({ status }: { status: CanvasStatus }) {
  return (
    <span
      data-status={status}
      aria-hidden="true"
      className={cn(
        'w-2 h-2 rounded-full flex-none',
        status === 'none' && 'bg-line-2',
        status === 'installed' && 'bg-amber',
        status === 'active' && 'bg-amber anim-dot-blink'
      )}
    />
  );
}

/** Level 0 — one card per GitHub account, its repositories as status dots. */
export function AccountNode({ data }: NodeProps<AccountFlowNode>) {
  const c = useContent().dashboard;
  const cc = c.canvas;
  const { account, onOpen, index } = data;
  const status = accountStatus(account);
  const activeCount = account.repos.reduce((n, r) => n + r.active_sessions, 0);
  const shown = account.repos.slice(0, MAX_REPO_DOTS);
  const overflow = account.repos.length - shown.length;

  return (
    <button
      type="button"
      onClick={() => onOpen(account.login)}
      aria-label={cc.openAccountAria.replace('{login}', account.login)}
      // Mount stagger on the card BODY (not the React-Flow wrapper): the wrapper
      // carries React Flow's positioning `transform`, so its enter cue must stay
      // opacity-only (`anim-overlay-in`); the body is an independent box, so
      // `anim-row-in`'s translateY animates safely here without fighting layout.
      style={{ width: ACCOUNT_NODE.width, height: ACCOUNT_NODE.height, ...staggerStyle(index ?? 0) }}
      className={cn(
        'anim-row-in text-left border rounded-card p-4 flex flex-col gap-2 cursor-pointer',
        // Surface (bg/border/shadow/glow) is owned by statusCardClasses; here we
        // just declare which paint properties ease on the hover accent.
        'transition-[box-shadow,border-color,background-color] duration-200',
        statusCardClasses(status)
      )}
    >
      <span className="flex items-center justify-between gap-2 w-full">
        <span className="font-mono text-eyebrow text-ghost uppercase">
          {account.kind === 'personal' ? c.repos.personalGroup : c.repos.orgGroup}
        </span>
        {account.owner && <Chip tone="amber">{cc.ownerBadge}</Chip>}
      </span>
      <span className="font-display font-semibold text-[15.5px] text-fg truncate w-full">
        {account.login}
      </span>
      <span className="flex items-center gap-2">
        <StatusBadge status={status} activeCount={activeCount} />
        <span className="font-mono text-[10.5px] text-ghost" title={!account.counts_complete ? cc.countsIncomplete : undefined}>
          {cc.repoCount.replace('{n}', String(account.repos.length))}
          {!account.counts_complete && ' ±'}
        </span>
      </span>
      <span className="flex flex-wrap items-center gap-1.5 mt-auto" data-testid="repo-dots">
        {shown.map((r) => (
          <RepoDot key={r.id} status={repoStatus(r)} />
        ))}
        {overflow > 0 && (
          <span className="font-mono text-[10px] text-ghost">
            {cc.moreRepos.replace('{n}', String(overflow))}
          </span>
        )}
      </span>
    </button>
  );
}

/** Level 1 — one card per repository of the selected account. */
export function RepoNode({ data }: NodeProps<RepoFlowNode>) {
  const c = useContent().dashboard;
  const cc = c.canvas;
  const { repo, onOpen, index } = data;
  const status = repoStatus(repo);

  return (
    <button
      type="button"
      onClick={() => onOpen(repo.owner, repo.name)}
      aria-label={cc.openRepoAria.replace('{repo}', `${repo.owner}/${repo.name}`)}
      // Body-level mount stagger; see AccountNode for why the transform-carrying
      // keyframe rides the body rather than the React-Flow wrapper.
      style={{ width: REPO_NODE.width, height: REPO_NODE.height, ...staggerStyle(index ?? 0) }}
      className={cn(
        'anim-row-in text-left border rounded-card p-3.5 flex flex-col gap-1.5 cursor-pointer',
        'transition-[box-shadow,border-color,background-color] duration-200',
        statusCardClasses(status)
      )}
    >
      <span className="flex items-center justify-between gap-2 w-full">
        <span className="font-display font-semibold text-[13.5px] text-fg truncate">
          {repo.name}
        </span>
        <Chip tone="neutral">{repo.private ? c.repos.private : c.repos.public}</Chip>
      </span>
      <StatusBadge status={status} activeCount={repo.active_sessions} />
      {repo.packages.length > 0 && (
        <span className="font-mono text-[10.5px] text-ghost truncate w-full mt-auto">
          {c.packages} · {repo.packages.length}
        </span>
      )}
    </button>
  );
}

/** The dynamic body of the level-2 card, keyed by fetch state so the parent's
 *  `FadeSwap` can crossfade loading→loaded (no shimmer→list hard cut) and give
 *  a failed fetch a terminal state instead of an endless shimmer.
 *
 *  UX decision (dead-card fix): rather than reducing this card to reclaim room
 *  for the sidebar — which would mean editing the sibling-owned `layout.ts`
 *  (`DETAIL_NODE.width`) and would break the sibling `flow.test.tsx` that asserts
 *  the session name/number render here — we make the card USEFUL in place: a
 *  compact status summary (active-count badge + total) tops the short session
 *  list, so the node conveys at-a-glance health the raw sidebar list does not.
 *  This is the lower-risk option (touches only files this item owns). */
function RepoDetailBody({
  bodyKey,
  sessions,
  installed,
}: {
  bodyKey: DetailBodyKey;
  sessions: RepoSessionsResponse | null;
  installed: boolean;
}) {
  const c = useContent().dashboard;
  const cc = c.canvas;

  switch (bodyKey) {
    case 'loading':
      return (
        <output aria-label={cc.loadingSidebar} className="flex flex-col gap-1.5">
          <span className="anim-shimmer h-3 rounded-chip w-3/4" />
          <span className="anim-shimmer h-3 rounded-chip w-1/2" />
        </output>
      );
    case 'failed':
      // Terminal: a failed level-2 fetch must not shimmer forever.
      return <span className="font-mono text-[11.5px] text-ghost">{cc.sessionsLoadFailed}</span>;
    case 'empty':
      return <span className="font-mono text-[11.5px] text-ghost">{c.noSessions}</span>;
    case 'ready': {
      // `bodyKey === 'ready'` already guarantees a non-empty payload.
      const list = sessions!.sessions;
      const activeCount = list.filter(sessionActive).length;
      return (
        <div className="flex flex-col gap-2.5">
          <div className="flex items-center gap-2">
            <StatusBadge status={repoDetailStatus(installed, sessions)} activeCount={activeCount} />
            <span className="font-mono text-[10.5px] text-ghost">
              {cc.sessionsTitle} · {list.length}
            </span>
          </div>
          <div className="flex flex-col gap-1">
            {list.slice(0, 6).map((s) => (
              // BUG B2: key is the stable session id, falling back to the trigger
              // NUMBER alone — the old `-${i}` positional suffix churned the key on
              // every reorder, forcing needless remounts of otherwise-stable rows.
              <span key={s.session_id ?? `t-${s.trigger.number}`} className="flex items-center gap-2">
                <RepoDot status={sessionActive(s) ? 'active' : 'none'} />
                <span className="font-mono text-[11.5px] text-dim truncate">
                  {s.name ?? c.invalidTrigger}
                </span>
                <span className="font-mono text-[10.5px] text-ghost">#{s.trigger.number}</span>
              </span>
            ))}
          </div>
        </div>
      );
    }
  }
}

type DetailBodyKey = 'loading' | 'failed' | 'empty' | 'ready';

/** Classify the level-2 fetch state into the `FadeSwap` key. A null payload is
 *  either a failed fetch (terminal) or still loading; a present payload is empty
 *  or ready. */
function detailBodyKey(sessions: RepoSessionsResponse | null, failed: boolean): DetailBodyKey {
  if (sessions == null) return failed ? 'failed' : 'loading';
  return sessions.sessions.length === 0 ? 'empty' : 'ready';
}

/** Level 2 — a single wide card for the opened repository. The sidebar holds
 *  the full detail; this node anchors the zoom target and shows a compact
 *  status summary (see `RepoDetailBody` for the dead-card rationale). */
export function RepoDetailNode({ data }: NodeProps<DetailFlowNode>) {
  const c = useContent().dashboard;
  const cc = c.canvas;
  const { owner, name, installed, sessions, sessionsFailed, index } = data;
  const bodyKey = detailBodyKey(sessions, sessionsFailed === true);

  return (
    <div
      // The single detail node is unmeasured, so its enter cue rides the body
      // like the grid cards. Index defaults to 0 (one node) → a plain fade.
      style={{ width: DETAIL_NODE.width, ...staggerStyle(index ?? 0) }}
      className={cn(
        'anim-row-in border rounded-card p-4 flex flex-col gap-2.5',
        'transition-[box-shadow,border-color,background-color] duration-200',
        statusCardClasses(repoDetailStatus(installed, sessions))
      )}
    >
      <div className="flex items-center justify-between gap-2">
        <span className="font-display font-semibold text-[15px] text-fg truncate">
          {owner}/{name}
        </span>
        {installed ? (
          <Chip tone="green">{c.installed}</Chip>
        ) : (
          <span className="font-mono text-[10.5px] text-ghost">{cc.statusNone}</span>
        )}
      </div>
      {/* Crossfade the body when the payload arrives (loading→ready) rather than
          hard-cutting the shimmer to the list. Reduced-motion-safe via FadeSwap. */}
      <FadeSwap k={bodyKey}>
        <RepoDetailBody bodyKey={bodyKey} sessions={sessions} installed={installed} />
      </FadeSwap>
    </div>
  );
}
