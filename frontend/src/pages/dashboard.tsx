import React, { useCallback, useEffect, useRef, useState } from 'react';
import { cn } from '@/lib/utils';
import { Eyebrow } from '@/components/layout/eyebrow';
import { Chip } from '@/components/ui/chip';
import { FIELD_INPUT } from '@/components/ui/field';
import { ConfirmDialog } from '@/components/modals/confirm-dialog';
import { CreateRepoModal } from '@/components/modals/create-repo-modal';
import type { UserRepo } from '@/components/modals/create-repo-modal';
import { useContent, useLang } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';

// ---- API shapes (mirror the backend DTOs) ----------------------------------

interface IssueView {
  number: number;
  title: string;
  state: string;
  author: string;
  labels: string[];
}
interface SessionGroup {
  session_id?: string | null;
  name?: string | null;
  work_label?: string | null;
  auto_merge?: boolean | null;
  environment?: string | null;
  packages: string[];
  invalid_reason?: string | null;
  status_labels: string[];
  trigger: IssueView;
  work_issues: IssueView[];
}
interface RepoView {
  owner: string;
  name: string;
  installation_id: number;
  sessions: SessionGroup[];
}
interface DashboardData {
  app_configured: boolean;
  installations: number;
  repos: RepoView[];
}
interface DashboardResponse {
  last_pulled_at_ms: number | null;
  dashboard: DashboardData | null;
}
interface PullJob {
  job_id: string;
  state: string; // running | done | error
  phase: string;
  done: number;
  total: number;
  error?: string | null;
}
interface InstallationView {
  account: string;
  installation_id: number;
  repository_selection: 'all' | 'selected';
}
interface UserReposResponse {
  app_slug: string | null;
  viewer: { login: string };
  orgs: string[];
  installations: InstallationView[];
  repos: UserRepo[];
}
type DashboardContent = ReturnType<typeof useContent>['dashboard'];
type ReposContent = DashboardContent['repos'];

/** Format an epoch-ms as Singapore time (the dashboard's canonical timezone). */
export function formatSgt(ms: number, lang: 'en' | 'zh'): string {
  try {
    const s = new Intl.DateTimeFormat(lang === 'zh' ? 'zh-CN' : 'en-GB', {
      timeZone: 'Asia/Singapore',
      dateStyle: 'medium',
      timeStyle: 'short',
    }).format(new Date(ms));
    return `${s} SGT`;
  } catch {
    return new Date(ms).toISOString();
  }
}

const delay = (ms: number) => new Promise<void>((r) => window.setTimeout(r, ms));

// ---- Small presentational pieces -------------------------------------------

function IssueRow({
  issue,
  openLabel,
  closedLabel,
}: {
  issue: IssueView;
  openLabel: string;
  closedLabel: string;
}) {
  const closed = issue.state === 'closed';
  return (
    <div className="flex items-center gap-2 py-1.5 text-[12.5px] min-w-0">
      <span
        className={cn('w-1.5 h-1.5 rounded-full flex-none', closed ? 'bg-ghost' : 'bg-green')}
        aria-hidden="true"
      />
      <span className="font-mono text-[11px] text-ghost flex-none">#{issue.number}</span>
      <span className="text-fg truncate min-w-0 flex-1">{issue.title}</span>
      <span className="font-mono text-[10.5px] text-ghost flex-none">
        {closed ? closedLabel : openLabel}
      </span>
    </div>
  );
}

function SessionCard({ s, d }: { s: SessionGroup; d: DashboardContent }) {
  const invalid = !!s.invalid_reason;
  return (
    <div className="border border-line rounded-card bg-bg p-4 flex flex-col gap-3 min-w-0">
      <div className="flex items-start justify-between gap-3 flex-wrap">
        <div className="min-w-0">
          <span className="font-display font-semibold text-[15px] text-fg">
            {invalid ? d.invalidTrigger : (s.name ?? '—')}
          </span>
          {s.session_id && (
            <span className="font-mono text-[10.5px] text-ghost ml-2 break-all">
              {s.session_id.slice(0, 8)}
            </span>
          )}
        </div>
        <div className="flex items-center gap-1.5 flex-wrap">
          {s.auto_merge && <Chip tone="green">{d.autoMerge}</Chip>}
          {s.status_labels.map((l) => (
            <Chip key={l} tone="amber">
              {l}
            </Chip>
          ))}
        </div>
      </div>

      {invalid ? (
        <p className="text-[12.5px] text-red leading-relaxed">{s.invalid_reason}</p>
      ) : (
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-[12px] text-dim">
          {s.work_label && (
            <span>
              {d.workLabel}: <code className="font-mono text-fg">{s.work_label}</code>
            </span>
          )}
          {s.environment && (
            <span>
              {d.environment}: <code className="font-mono text-fg">{s.environment}</code>
            </span>
          )}
        </div>
      )}

      {s.packages.length > 0 && (
        <div className="flex flex-col gap-1">
          <span className="font-mono text-eyebrow text-ghost uppercase">{d.packages}</span>
          <div className="flex flex-col gap-0.5">
            {s.packages.map((p) => (
              <code key={p} className="font-mono text-[11.5px] text-dim break-all">
                {p}
              </code>
            ))}
          </div>
        </div>
      )}

      <div className="border-t border-line pt-2 flex flex-col">
        <span className="font-mono text-eyebrow text-ghost uppercase mb-1">{d.trigger}</span>
        <IssueRow issue={s.trigger} openLabel={d.open} closedLabel={d.closed} />
        {s.work_issues.length > 0 && (
          <>
            <span className="font-mono text-eyebrow text-ghost uppercase mt-2 mb-1">
              {d.workIssues} · {s.work_issues.length}
            </span>
            <div className="flex flex-col divide-y divide-[color-mix(in_oklab,var(--line)_55%,transparent)]">
              {s.work_issues.map((w) => (
                <IssueRow key={w.number} issue={w} openLabel={d.open} closedLabel={d.closed} />
              ))}
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function RepoRow({
  repo,
  appSlug,
  rc,
  highlight = false,
  installed_via_installation = false,
}: {
  repo: UserRepo;
  appSlug: string | null;
  rc: ReposContent;
  highlight?: boolean;
  /** True when the account has an installation — adding/removing this repo is
   * done on that installation's GitHub settings page (the group's Manage link),
   * since GitHub allows per-repo selection changes only there, not via our
   * App user-to-server token. Drives the explanatory tooltip. */
  installed_via_installation?: boolean;
}) {
  return (
    <div
      className={cn(
        'flex items-center gap-2 py-2 px-2 -mx-2 rounded-control text-[12.5px] min-w-0',
        'transition-colors hover:bg-[color-mix(in_oklab,var(--raise-2)_80%,transparent)]',
        highlight && 'anim-repo-pulse'
      )}
    >
      <a
        href={`https://github.com/${repo.owner}/${repo.name}`}
        target="_blank"
        rel="noreferrer"
        className="font-mono text-[12px] text-fg hover:text-amber transition-colors truncate min-w-0"
      >
        {`${repo.owner}/${repo.name}`}
      </a>
      <Chip tone="neutral">{repo.private ? rc.private : rc.public}</Chip>
      {repo.org && <Chip tone="amber">{rc.org}</Chip>}
      <span className="flex-1" aria-hidden="true" />
      {repo.installed ? (
        <span
          className="font-mono text-[11px] text-green flex-none"
          title={installed_via_installation ? rc.manageRepoHint : undefined}
        >{`✓ ${rc.installed}`}</span>
      ) : (
        appSlug != null && (
          <a
            href={`https://github.com/apps/${appSlug}/installations/new`}
            target="_blank"
            rel="noreferrer"
            title={repo.admin ? undefined : rc.nonAdminHint}
            className="font-ui font-semibold text-[11.5px] bg-amber text-amber-ink rounded-control px-3 py-1 transition-colors hover:brightness-[1.06] flex-none"
          >
            {rc.install}
          </a>
        )
      )}
    </div>
  );
}

// ---- Repositories section (grouped + searchable + create flow) --------------

interface RepoGroup {
  owner: string;
  personal: boolean;
  repos: UserRepo[];
}

/** Group repos by owner: the viewer's personal group first (always present),
 *  then org groups alphabetically. Orgs from `orgs` with zero repos are kept
 *  as empty groups so they surface as creation targets. */
function buildGroups(viewerLogin: string, orgs: string[], repos: UserRepo[]): RepoGroup[] {
  const byOwner = new Map<string, UserRepo[]>();
  for (const repo of repos) {
    const list = byOwner.get(repo.owner);
    if (list) list.push(repo);
    else byOwner.set(repo.owner, [repo]);
  }
  const orgOwners = new Set<string>(orgs);
  for (const owner of byOwner.keys()) {
    if (owner !== viewerLogin) orgOwners.add(owner);
  }
  const sorted = [...orgOwners].sort((a, b) => a.toLowerCase().localeCompare(b.toLowerCase()));
  return [
    { owner: viewerLogin, personal: true, repos: byOwner.get(viewerLogin) ?? [] },
    ...sorted.map((owner) => ({ owner, personal: false, repos: byOwner.get(owner) ?? [] })),
  ];
}


/** Pending danger action driving the shared ConfirmDialog. */
type ConfirmTarget = { kind: 'uninstall'; owner: string };

/** Exact GitHub settings page for an installation on this account. */
function manageUrl(owner: string, personal: boolean, installationId: number): string {
  return personal
    ? `https://github.com/settings/installations/${installationId}`
    : `https://github.com/organizations/${owner}/settings/installations/${installationId}`;
}

function ReposSection() {
  const rc = useContent().dashboard.repos;
  const { apiFetch } = useAuth();

  const [data, setData] = useState<UserReposResponse | null>(null);
  const [loadError, setLoadError] = useState(false);
  const [tick, setTick] = useState(0);
  const [search, setSearch] = useState('');
  const [showCreate, setShowCreate] = useState(false);
  // `owner/name` of the repo created via the modal — highlighted after re-fetch.
  const [createdKey, setCreatedKey] = useState<string | null>(null);
  // Pending danger action (account uninstall / per-repo remove), if any.
  const [confirm, setConfirm] = useState<ConfirmTarget | null>(null);

  // Load the user's repositories + App installation status on mount; Refresh
  // (and a successful create) re-runs it by bumping `tick`.
  useEffect(() => {
    let active = true;
    setLoadError(false);
    apiFetch('/api/v1/repos')
      .then((r) =>
        r.ok ? (r.json() as Promise<UserReposResponse>) : Promise.reject(new Error(String(r.status)))
      )
      .then((j) => {
        if (active) setData(j);
      })
      .catch(() => {
        if (active) setLoadError(true);
      });
    return () => {
      active = false;
    };
  }, [apiFetch, tick]);

  const onCreated = useCallback((repo: UserRepo) => {
    setShowCreate(false);
    setCreatedKey(`${repo.owner}/${repo.name}`);
    setSearch(''); // make sure the new repo is visible whatever was typed
    setTick((t) => t + 1);
  }, []);

  const onConfirmClose = useCallback(() => setConfirm(null), []);
  const onConfirmDone = useCallback(() => {
    setConfirm(null);
    setTick((t) => t + 1); // the danger action changed installation state — re-fetch
  }, []);

  const query = search.trim().toLowerCase();
  const groups = data == null ? [] : buildGroups(data.viewer.login, data.orgs, data.repos);
  // App installation per account login (personal user or organization).
  const installations = data?.installations ?? [];
  const installationFor = (owner: string) =>
    installations.find((i) => i.account === owner) ?? null;
  // Under an active search, a group keeps only matching rows and collapses
  // away entirely when nothing matches (empty creation-target groups too).
  const visibleGroups = groups
    .map((g) => ({
      ...g,
      shown: query
        ? g.repos.filter((r) => `${r.owner}/${r.name}`.toLowerCase().includes(query))
        : g.repos,
    }))
    .filter((g) => !query || g.shown.length > 0);
  const hasAnyTarget = data != null && (data.repos.length > 0 || data.orgs.length > 0);

  // Global row index driving the staggered entrance (capped delay).
  let animIdx = 0;
  const nextStagger = () =>
    ({ '--stagger': `${Math.min(animIdx++ * 30, 240)}ms` }) as React.CSSProperties;

  return (
    <section className="border border-line rounded-panel bg-raise p-8 max-[600px]:p-5 flex flex-col gap-4">
      <div className="flex items-center justify-between gap-3 flex-wrap">
        <h2 className="font-display font-semibold text-[18px] text-fg">{rc.title}</h2>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => setShowCreate(true)}
            disabled={data == null}
            className={cn(
              'font-ui font-semibold text-[12px] rounded-control px-3 py-1.5 transition-colors',
              data == null
                ? 'bg-amber/50 text-amber-ink/60 cursor-not-allowed'
                : 'bg-amber text-amber-ink hover:brightness-[1.06] cursor-pointer'
            )}
          >
            {rc.newRepo}
          </button>
          <button
            type="button"
            onClick={() => setTick((t) => t + 1)}
            className="font-ui font-semibold text-[12px] border border-line rounded-control px-3 py-1.5 text-dim hover:text-fg transition-colors cursor-pointer"
          >
            {rc.refresh}
          </button>
        </div>
      </div>

      {loadError ? (
        <div className="border border-line border-l-2 border-l-red rounded-card bg-[color-mix(in_oklab,var(--raise)_55%,transparent)] px-4 py-3 text-[13px] text-dim">
          {rc.loadFailed}
        </div>
      ) : data == null ? (
        <p className="font-mono text-[12px] text-ghost">{rc.loading}</p>
      ) : (
        <>
          {data.app_slug == null && (
            <p className="font-mono text-[12px] text-ghost">{rc.appNotConfigured}</p>
          )}
          {!hasAnyTarget ? (
            <p className="font-mono text-[12.5px] text-ghost">{rc.empty}</p>
          ) : (
            <>
              {data.repos.length > 0 && (
                <input
                  type="search"
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                  placeholder={rc.searchPlaceholder}
                  aria-label={rc.searchPlaceholder}
                  className={FIELD_INPUT}
                />
              )}
              {query && visibleGroups.length === 0 ? (
                <p className="font-mono text-[12.5px] text-ghost">{rc.searchEmpty}</p>
              ) : (
                <div className="flex flex-col gap-5">
                  {visibleGroups.map((g) => {
                    const inst = installationFor(g.owner);
                    return (
                    <section key={g.owner} className="flex flex-col gap-1">
                      <div className="flex items-center gap-2.5 flex-wrap">
                        <span className="font-mono text-eyebrow text-ghost uppercase">
                          {g.personal ? rc.personalGroup : rc.orgGroup}
                        </span>
                        <h3 className="font-display font-semibold text-[14.5px] text-fg">
                          {g.owner}
                        </h3>
                        <span className="flex-1" aria-hidden="true" />
                        <span className="font-mono text-[11px] text-ghost">
                          {rc.groupCounts
                            .replace(
                              '{installed}',
                              String(g.repos.filter((r) => r.installed).length)
                            )
                            .replace('{total}', String(g.repos.length))}
                        </span>
                        {inst != null ? (
                          <span className="flex items-center gap-1.5">
                            <a
                              href={manageUrl(g.owner, g.personal, inst.installation_id)}
                              target="_blank"
                              rel="noreferrer"
                              className="font-ui font-semibold text-[11px] border border-line rounded-control px-2.5 py-1 text-dim hover:text-fg transition-colors"
                            >
                              {rc.manage}
                            </a>
                            <button
                              type="button"
                              onClick={() => setConfirm({ kind: 'uninstall', owner: g.owner })}
                              className="font-ui font-semibold text-[11px] border border-line rounded-control px-2.5 py-1 text-red transition-colors hover:border-[color-mix(in_oklab,var(--red)_45%,var(--line))] cursor-pointer"
                            >
                              {rc.uninstall}
                            </button>
                          </span>
                        ) : (
                          data.app_slug != null && (
                            <span className="flex items-center gap-2">
                              <span className="font-ui text-[11px] text-ghost">
                                {rc.connectHint}
                              </span>
                              <a
                                href={`https://github.com/apps/${data.app_slug}/installations/new`}
                                target="_blank"
                                rel="noreferrer"
                                className="font-ui font-semibold text-[11.5px] bg-amber text-amber-ink rounded-control px-3 py-1 transition-colors hover:brightness-[1.06]"
                              >
                                {rc.connect}
                              </a>
                            </span>
                          )
                        )}
                      </div>
                      {g.repos.length === 0 ? (
                        <p className="font-mono text-[12px] text-ghost italic py-1">
                          {rc.groupEmpty}
                        </p>
                      ) : (
                        <div className="flex flex-col divide-y divide-[color-mix(in_oklab,var(--line)_55%,transparent)]">
                          {g.shown.map((repo) => {
                            const key = `${repo.owner}/${repo.name}`;
                            const isNew = key === createdKey;
                            return (
                              <div key={key} className="anim-row-in" style={nextStagger()}>
                                <RepoRow
                                  repo={repo}
                                  appSlug={data.app_slug}
                                  rc={rc}
                                  highlight={isNew}
                                  installed_via_installation={inst != null}
                                />
                                {isNew && !repo.installed && data.app_slug != null && (
                                  <div className="mb-2 flex items-center gap-3 flex-wrap border rounded-card px-3 py-2 text-[12.5px] text-dim border-[color-mix(in_oklab,var(--amber)_35%,var(--line))] bg-[color-mix(in_oklab,var(--amber)_8%,transparent)]">
                                    <span>{rc.createdNextStep}</span>
                                    <a
                                      href={`https://github.com/apps/${data.app_slug}/installations/new`}
                                      target="_blank"
                                      rel="noreferrer"
                                      className="font-ui font-semibold text-[11.5px] bg-amber text-amber-ink rounded-control px-3 py-1 transition-colors hover:brightness-[1.06] flex-none"
                                    >
                                      {rc.install}
                                    </a>
                                  </div>
                                )}
                              </div>
                            );
                          })}
                        </div>
                      )}
                    </section>
                    );
                  })}
                </div>
              )}
            </>
          )}
        </>
      )}

      {showCreate && data != null && (
        <CreateRepoModal
          viewerLogin={data.viewer.login}
          orgs={data.orgs}
          rc={rc}
          onClose={() => setShowCreate(false)}
          onCreated={onCreated}
        />
      )}

      {confirm != null && (
        <ConfirmDialog
          title={rc.uninstallConfirmTitle.replace('{owner}', confirm.owner)}
          body={rc.uninstallConfirmBody.replace('{owner}', confirm.owner)}
          confirmLabel={rc.uninstallConfirm}
          pendingLabel={rc.uninstallPending}
          cancelLabel={rc.cancel}
          path={`/api/v1/installations/${encodeURIComponent(confirm.owner)}`}
          fallbackError={rc.uninstallFailed}
          onClose={onConfirmClose}
          onDone={onConfirmDone}
        />
      )}
    </section>
  );
}

// ---- Page -------------------------------------------------------------------

export function Dashboard() {
  const c = useContent();
  const d = c.dashboard;
  const { lang } = useLang();
  const { configured, isAuthenticated, error, signIn, apiFetch } = useAuth();

  const [data, setData] = useState<DashboardResponse | null>(null);
  const [pulling, setPulling] = useState(false);
  const [progress, setProgress] = useState<PullJob | null>(null);
  const [pullError, setPullError] = useState<string | null>(null);
  const cancelled = useRef(false);

  useEffect(() => {
    document.title = d.metaTitle;
  }, [d.metaTitle]);

  // Load the cached dashboard on mount (never recomputes; that's the Update button).
  useEffect(() => {
    if (!isAuthenticated || !configured) return;
    let active = true;
    apiFetch('/api/v1/dashboard')
      .then((r) =>
        r.ok ? (r.json() as Promise<DashboardResponse>) : Promise.reject(new Error(String(r.status)))
      )
      .then((j) => {
        if (active) setData(j);
      })
      .catch(() => {
        if (active) setData({ last_pulled_at_ms: null, dashboard: null });
      });
    return () => {
      active = false;
    };
  }, [isAuthenticated, configured, apiFetch]);

  useEffect(
    () => () => {
      cancelled.current = true;
    },
    []
  );

  const onUpdate = useCallback(async () => {
    setPullError(null);
    setProgress(null);
    setPulling(true);
    cancelled.current = false;
    try {
      const res = await apiFetch('/api/v1/dashboard/pull', { method: 'POST' });
      if (!res.ok) throw new Error(String(res.status));
      const job = (await res.json()) as PullJob;
      setProgress(job);
      // Poll the job until it reaches a terminal state.
      for (;;) {
        if (cancelled.current) return;
        await delay(1000);
        const r = await apiFetch(`/api/v1/dashboard/pull/${encodeURIComponent(job.job_id)}`);
        if (!r.ok) throw new Error(String(r.status));
        const p = (await r.json()) as PullJob;
        setProgress(p);
        if (p.state === 'done') {
          const dr = await apiFetch('/api/v1/dashboard');
          if (dr.ok) setData((await dr.json()) as DashboardResponse);
          break;
        }
        if (p.state === 'error') {
          setPullError(p.error || d.updateFailed);
          break;
        }
      }
    } catch {
      setPullError(d.updateFailed);
    } finally {
      if (!cancelled.current) setPulling(false);
    }
  }, [apiFetch, d.updateFailed]);

  const header = (
    <header>
      <Eyebrow>{d.eyebrow}</Eyebrow>
      <h1 className="mt-5 font-display font-bold text-[clamp(28px,4vw,40px)] leading-[1.1] tracking-[-0.02em] text-fg">
        {d.title}
      </h1>
      <p className="mt-5 text-[15px] leading-relaxed text-dim max-w-[68ch]">{d.lede}</p>
    </header>
  );

  if (!isAuthenticated) {
    return (
      <div className="flex flex-col gap-8 max-w-[960px]">
        {header}
        {error && (
          <div className="border border-line border-l-2 border-l-red rounded-card bg-[color-mix(in_oklab,var(--raise)_55%,transparent)] px-4 py-3 text-[13px] text-dim">
            {d.authError}
          </div>
        )}
        <section className="border border-line rounded-panel bg-raise p-8 max-[600px]:p-5 flex flex-col items-start gap-4">
          <h2 className="font-display font-semibold text-[20px] text-fg">{d.signInTitle}</h2>
          <p className="text-[14px] leading-relaxed text-dim max-w-[56ch]">{d.signInBody}</p>
          {configured ? (
            <button
              type="button"
              onClick={signIn}
              className="font-ui font-semibold text-[13.5px] bg-amber text-amber-ink rounded-control px-5 py-2.5 transition-colors hover:brightness-[1.06] cursor-pointer"
            >
              {c.auth.signIn}
            </button>
          ) : (
            <p className="font-mono text-[12px] text-ghost">{d.notConfigured}</p>
          )}
        </section>
      </div>
    );
  }

  if (!configured) {
    return (
      <div className="flex flex-col gap-8 max-w-[960px]">
        {header}
        <p className="font-mono text-[12px] text-ghost">{d.notConfigured}</p>
      </div>
    );
  }

  const lastMs = data?.last_pulled_at_ms ?? null;
  const repos = data?.dashboard?.repos ?? null;

  return (
    <div className="flex flex-col gap-8 max-w-[960px]">
      {header}

      <div className="flex items-center gap-4 flex-wrap pb-3.5 border-b border-line">
        <button
          type="button"
          onClick={onUpdate}
          disabled={pulling}
          className={cn(
            'font-ui font-semibold text-[12.5px] rounded-control px-4 py-2 transition-colors',
            pulling
              ? 'bg-amber/50 text-amber-ink/60 cursor-not-allowed'
              : 'bg-amber text-amber-ink hover:brightness-[1.06] cursor-pointer'
          )}
        >
          {pulling ? d.updating : d.update}
        </button>
        <span className="font-mono text-[11.5px] text-ghost">
          {d.lastUpdated}: {lastMs ? formatSgt(lastMs, lang) : d.never}
        </span>
        <span className="font-mono text-[11px] text-ghost max-[600px]:w-full">· {d.updatesNote}</span>
      </div>

      {pullError && (
        <div className="border border-line border-l-2 border-l-red rounded-card bg-[color-mix(in_oklab,var(--raise)_55%,transparent)] px-4 py-3 text-[13px] text-dim">
          {pullError}
        </div>
      )}

      {pulling && (
        <div className="flex flex-col gap-2 border border-line rounded-card bg-raise p-4">
          <div className="flex items-center justify-between text-[12.5px] text-dim">
            <span>{d.loadingTitle}</span>
            {progress && progress.total > 0 && (
              <span className="font-mono text-[11.5px] text-ghost">
                {d.reposScanned
                  .replace('{done}', String(progress.done))
                  .replace('{total}', String(progress.total))}
              </span>
            )}
          </div>
          <div className="h-2 rounded-full bg-line-2 overflow-hidden">
            <div
              className="h-full bg-amber transition-[width] duration-300"
              style={{
                width: `${progress && progress.total > 0 ? Math.round((progress.done / progress.total) * 100) : 0}%`,
              }}
            />
          </div>
        </div>
      )}

      {!pulling && (
        <>
          {repos == null ? (
            <section className="border border-line rounded-panel bg-raise p-8 max-[600px]:p-5 flex flex-col items-start gap-3">
              <h2 className="font-display font-semibold text-[18px] text-fg">{d.firstVisitTitle}</h2>
              <p className="text-[14px] leading-relaxed text-dim max-w-[56ch]">{d.firstVisitBody}</p>
            </section>
          ) : repos.length === 0 ? (
            <p className="font-mono text-[12.5px] text-ghost py-6">{d.noRepos}</p>
          ) : (
            <div className="flex flex-col gap-6">
              {repos.map((repo) => (
                <section key={`${repo.owner}/${repo.name}`} className="flex flex-col gap-3">
                  <div className="flex items-center gap-3 flex-wrap">
                    <h2 className="font-display font-semibold text-[17px] text-fg">
                      {repo.owner}/{repo.name}
                    </h2>
                    <Chip tone="green">{d.installed}</Chip>
                    <span className="font-mono text-[11px] text-ghost">· {repo.sessions.length}</span>
                  </div>
                  {repo.sessions.length === 0 ? (
                    <p className="font-mono text-[12px] text-ghost italic">{d.noSessions}</p>
                  ) : (
                    <div className="grid grid-cols-1 gap-3">
                      {repo.sessions.map((s, i) => (
                        <SessionCard
                          key={s.session_id ?? `${repo.name}-${s.trigger.number}-${i}`}
                          s={s}
                          d={d}
                        />
                      ))}
                    </div>
                  )}
                </section>
              ))}
            </div>
          )}
        </>
      )}

      <ReposSection />
    </div>
  );
}
