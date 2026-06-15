import { Link, useNavigate } from 'react-router-dom';
import { cn } from '@/lib/utils';
import { PostureChip } from '@/components/status/posture-chip';
import React, { useState } from 'react';
import { GoalView, mapRepoTargetError } from '@/lib/api/goals';
import { goalStatusPresentation } from '@/lib/api/goal-status';
import { useTriggerGoal, useUpdateGoal, useDeleteGoal } from '@/lib/hooks/useGoals';
import { useSessionRegistry } from '@/lib/hooks/session-registry';
import { toast } from '@/components/primitives/toaster';
import { Dialog, DialogContent, DialogTitle, DialogDescription, DialogTrigger, DialogClose } from '@/components/primitives/dialog';

export interface LifecycleEvent {
  name: string;
  timestamp?: string;
  body?: string;
  marker?: string;
  trustedBy?: string;
  isCurrent?: boolean;
  type?: 'approve' | 'converge' | 'now';
}

export interface GoalProps {
  goal?: GoalView;
  goalId?: string;
  title?: string;
  state?: string;
  version?: string;
  headSha?: string;
  branch?: string;
  blocksGoalId?: string;
  lifecycleEvents?: LifecycleEvent[];
  deliveries?: {
    status: 'ACK' | 'LEASED';
    name: string;
    gen: number;
    state: string;
    timeLeft?: string;
    sourceRef?: string;
  }[];
  runs?: {
    exitCode: number | null;
    action: string;
    duration: string;
    permits?: number;
  }[];
  pr?: {
    number: number;
    href: string;
  };
  isReal?: boolean;
  mergeGate?: {
    reviewApproved?: 'ok' | 'fail' | 'unknown';
    headBound?: 'ok' | 'fail' | 'unknown';
    ciGreen?: 'ok' | 'fail' | 'unknown';
    mergeable?: 'ok' | 'fail' | 'unknown';
    posture?: 'ok' | 'fail' | 'unknown';
  };
  consensus?: {
    summary: string;
    passes?: boolean;
  };
}

export function Goal({
  goal,
  goalId: initialGoalId = '—',
  title: initialTitle,
  state: initialState = 'unknown',
  version = 'unknown',
  headSha = 'unknown',
  branch = 'unknown',
  blocksGoalId,
  lifecycleEvents = [],
  deliveries = [],
  runs = [],
  pr,
  isReal = false,
  mergeGate,
  consensus,
}: GoalProps) {
  const navigate = useNavigate();
  const { registerSession } = useSessionRegistry();

  // Mutations
  const triggerMutation = useTriggerGoal();
  const updateMutation = useUpdateGoal();
  const deleteMutation = useDeleteGoal();

  // Dialog/Modal local states
  const [isEditDialogOpen, setIsEditDialogOpen] = useState(false);
  const [isDeleteDialogOpen, setIsDeleteDialogOpen] = useState(false);
  
  // Edit Form states
  const [editTitle, setEditTitle] = useState(goal ? goal.title : '');
  const [editDescription, setEditDescription] = useState(goal ? goal.description : '');
  const [editPackages, setEditPackages] = useState(goal ? goal.package_names.join(', ') : '');
  const [editRepoOwner, setEditRepoOwner] = useState(goal && goal.repo ? goal.repo.owner : '');
  const [editRepoName, setEditRepoName] = useState(goal && goal.repo ? goal.repo.name : '');
  const [editClearRepo, setEditClearRepo] = useState(false);

  // Sync edit form states when goal changes
  React.useEffect(() => {
    if (goal) {
      setEditTitle(goal.title);
      setEditDescription(goal.description);
      setEditPackages(goal.package_names.join(', '));
      setEditRepoOwner(goal.repo ? goal.repo.owner : '');
      setEditRepoName(goal.repo ? goal.repo.name : '');
      setEditClearRepo(false);
    }
  }, [goal]);

  // Derive variables from goal (if present) or fallback
  const isHosted = !!goal;
  const goalId = goal ? goal.id : initialGoalId;
  const title = goal ? goal.title : initialTitle;
  const repoStr = goal && goal.repo ? `${goal.repo.owner}/${goal.repo.name}` : undefined;
  const packageNames = goal ? goal.package_names : [];
  const activeSessionId = goal ? goal.active_session_id : null;

  // Derive status/state presentation
  const pres = goal ? goalStatusPresentation(goal.status) : null;
  const state = goal ? goal.status : initialState;

  const hasData = title !== undefined || lifecycleEvents.length > 0 || isHosted;

  const handleTrigger = async () => {
    if (!goal) return;
    try {
      const res = await triggerMutation.mutateAsync({
        id: goal.id,
        req: {
          repo_mode: 'existing',
          repo: goal.repo,
        },
      });
      if (res.session_id) {
        goal.package_names.forEach((pkgName) => {
          registerSession(pkgName, res.session_id);
        });
        toast({
          title: 'Goal Triggered',
          description: (
            <span>
              Goal triggered successfully. Session{' '}
              <Link to="/settings" className="underline font-mono">
                {res.session_id}
              </Link>{' '}
              registered.
            </span>
          ),
        });
      }
    } catch (err) {
      const msg = mapRepoTargetError(err, 'trigger');
      toast({
        title: 'Trigger Failed',
        description: msg,
      });
    }
  };

  const handleEditSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!goal) return;
    
    const pkgs = editPackages.split(',').map((p) => p.trim()).filter(Boolean);
    const repo = (!editClearRepo && editRepoOwner && editRepoName)
      ? { owner: editRepoOwner.trim(), name: editRepoName.trim() }
      : null;
      
    try {
      await updateMutation.mutateAsync({
        id: goal.id,
        req: {
          title: editTitle.trim(),
          description: editDescription.trim(),
          package_names: pkgs,
          repo,
          clear_repo: editClearRepo ? true : undefined,
        },
      });
      setIsEditDialogOpen(false);
      toast({
        title: 'Goal Updated',
        description: 'Goal updated successfully.',
      });
    } catch (err) {
      toast({
        title: 'Update Failed',
        description: err instanceof Error ? err.message : 'An error occurred',
      });
    }
  };

  const handleDelete = async () => {
    if (!goal) return;
    try {
      await deleteMutation.mutateAsync(goal.id);
      setIsDeleteDialogOpen(false);
      toast({
        title: 'Goal Deleted',
        description: 'Goal deleted successfully.',
      });
      navigate('/goals');
    } catch (err) {
      toast({
        title: 'Delete Failed',
        description: err instanceof Error ? err.message : 'An error occurred',
      });
    }
  };

  return (
    <div className="flex flex-col gap-6">
      {/* Back navigation row */}
      <div className="flex items-center justify-between gap-4 flex-wrap">
        <Link
          to="/goals"
          className="font-mono text-[11.5px] text-ghost no-underline hover:text-dim transition-colors"
        >
          ← Goals · list
        </Link>
        {isHosted ? (
          <div className="flex items-center gap-2">
            <button
              onClick={handleTrigger}
              disabled={triggerMutation.isPending}
              className="font-ui font-semibold text-[12.5px] bg-amber text-amber-ink border-0 rounded-control px-3.5 py-[7px] cursor-pointer hover:brightness-[1.06] transition-colors disabled:opacity-50 disabled:cursor-not-allowed flex-shrink-0"
            >
              {triggerMutation.isPending ? 'Triggering...' : 'Trigger'}
            </button>
            <Dialog open={isEditDialogOpen} onOpenChange={setIsEditDialogOpen}>
              <DialogTrigger asChild>
                <button className="font-ui font-semibold text-[12.5px] bg-raise border border-line-2 text-fg rounded-control px-3.5 py-[7px] cursor-pointer hover:bg-raise-2 transition-colors flex-shrink-0">
                  Edit
                </button>
              </DialogTrigger>
              <DialogContent>
                <DialogTitle>Edit Goal</DialogTitle>
                <DialogDescription>Modify hosted goal properties.</DialogDescription>
                <form onSubmit={handleEditSubmit} className="flex flex-col gap-4 mt-4 text-left">
                  <div className="flex flex-col gap-1">
                    <label htmlFor="edit-title" className="text-[12px] text-faint font-medium">Title</label>
                    <input
                      id="edit-title"
                      type="text"
                      value={editTitle}
                      onChange={(e) => setEditTitle(e.target.value)}
                      className="bg-bg border border-line rounded-control p-2 text-[13px] text-fg focus:outline-none focus:border-amber"
                      required
                    />
                  </div>
                  <div className="flex flex-col gap-1">
                    <label htmlFor="edit-desc" className="text-[12px] text-faint font-medium">Description</label>
                    <textarea
                      id="edit-desc"
                      value={editDescription}
                      onChange={(e) => setEditDescription(e.target.value)}
                      className="bg-bg border border-line rounded-control p-2 text-[13px] text-fg focus:outline-none focus:border-amber min-h-[80px]"
                    />
                  </div>
                  <div className="flex flex-col gap-1">
                    <label htmlFor="edit-packages" className="text-[12px] text-faint font-medium">Packages (comma-separated)</label>
                    <input
                      id="edit-packages"
                      type="text"
                      value={editPackages}
                      onChange={(e) => setEditPackages(e.target.value)}
                      className="bg-bg border border-line rounded-control p-2 text-[13px] text-fg focus:outline-none focus:border-amber"
                      placeholder="e.g. pkg1, pkg2"
                      required
                    />
                  </div>
                  <div className="flex flex-col gap-1">
                    <label htmlFor="edit-repo-owner" className="text-[12px] text-faint font-medium">Target GitHub Repo (owner/name)</label>
                    <div className="flex gap-2">
                      <input
                        id="edit-repo-owner"
                        type="text"
                        value={editRepoOwner}
                        onChange={(e) => {
                          setEditRepoOwner(e.target.value);
                          setEditClearRepo(false);
                        }}
                        placeholder="owner"
                        className="bg-bg border border-line rounded-control p-2 text-[13px] text-fg focus:outline-none focus:border-amber flex-1"
                        disabled={editClearRepo}
                      />
                      <span className="text-ghost self-center">/</span>
                      <input
                        id="edit-repo-name"
                        type="text"
                        value={editRepoName}
                        onChange={(e) => {
                          setEditRepoName(e.target.value);
                          setEditClearRepo(false);
                        }}
                        placeholder="repo"
                        className="bg-bg border border-line rounded-control p-2 text-[13px] text-fg focus:outline-none focus:border-amber flex-1"
                        disabled={editClearRepo}
                      />
                    </div>
                  </div>
                  <div className="flex items-center gap-2 mt-1">
                    <input
                      type="checkbox"
                      id="clearRepoCheckbox"
                      checked={editClearRepo}
                      onChange={(e) => {
                        setEditClearRepo(e.target.checked);
                        if (e.target.checked) {
                          setEditRepoOwner('');
                          setEditRepoName('');
                        }
                      }}
                      className="accent-amber cursor-pointer"
                    />
                    <label htmlFor="clearRepoCheckbox" className="text-[12px] text-dim cursor-pointer select-none">
                      Clear repository connection
                    </label>
                  </div>
                  <div className="flex gap-2 justify-end mt-4">
                    <DialogClose asChild>
                      <button type="button" className="text-[12.5px] font-semibold bg-raise border border-line-2 text-fg rounded-control px-4 py-2 cursor-pointer hover:bg-raise-2 transition-colors">
                        Cancel
                      </button>
                    </DialogClose>
                    <button
                      type="submit"
                      disabled={updateMutation.isPending}
                      className="text-[12.5px] font-semibold bg-amber text-amber-ink rounded-control px-4 py-2 cursor-pointer hover:brightness-[1.06] transition-colors disabled:opacity-50"
                    >
                      {updateMutation.isPending ? 'Saving...' : 'Save'}
                    </button>
                  </div>
                </form>
              </DialogContent>
            </Dialog>
            <Dialog open={isDeleteDialogOpen} onOpenChange={setIsDeleteDialogOpen}>
              <DialogTrigger asChild>
                <button className="font-ui font-semibold text-[12.5px] bg-red text-fg border-0 rounded-control px-3.5 py-[7px] cursor-pointer hover:brightness-[1.06] transition-colors flex-shrink-0">
                  Delete
                </button>
              </DialogTrigger>
              <DialogContent>
                <DialogTitle>Delete Goal</DialogTitle>
                <DialogDescription>
                  Are you sure you want to delete this goal? This action cannot be undone.
                </DialogDescription>
                <div className="flex gap-2 justify-end mt-6">
                  <DialogClose asChild>
                    <button className="text-[12.5px] font-semibold bg-raise border border-line-2 text-fg rounded-control px-4 py-2 cursor-pointer hover:bg-raise-2 transition-colors">
                      Cancel
                    </button>
                  </DialogClose>
                  <button
                    onClick={handleDelete}
                    disabled={deleteMutation.isPending}
                    className="text-[12.5px] font-semibold bg-red text-fg rounded-control px-4 py-2 cursor-pointer hover:brightness-[1.06] transition-colors disabled:opacity-50"
                  >
                    {deleteMutation.isPending ? 'Deleting...' : 'Delete'}
                  </button>
                </div>
              </DialogContent>
            </Dialog>
          </div>
        ) : (
          <button
            disabled
            className="font-ui font-semibold text-[12.5px] bg-amber/50 text-amber-ink/50 cursor-not-allowed rounded-control px-3.5 py-[7px] opacity-50 select-none flex-shrink-0"
          >
            + New goal
          </button>
        )}
      </div>

      {/* Decision Header */}
      <div className="pb-[22px] border-b border-line">
        <div className="flex items-baseline gap-[11px] mt-2.5 mb-3 flex-wrap">
          {title ? (
            <h1 className="font-display font-semibold text-[23px] tracking-[-0.01em] text-fg leading-[1.1]">
              {title}
            </h1>
          ) : (
            <span className="font-display font-semibold text-[23px] tracking-[-0.01em] text-ghost select-none">
              —
            </span>
          )}
          <span className="font-mono text-[14px] text-faint select-none">
            #{goalId}
            {pr && (
              <>
                {' · '}
                <a
                  href={pr.href}
                  target="_blank"
                  rel="noreferrer"
                  className="text-faint hover:text-dim hover:underline"
                >
                  PR #{pr.number}
                </a>
              </>
            )}
          </span>
        </div>

        <div className="flex items-center gap-3.5 flex-wrap text-dim text-[12px]">
          {/* State Pill */}
          {isHosted && pres ? (
            <div className={cn(
              "inline-flex items-center gap-[7px] font-ui font-semibold text-[11.5px] tracking-[0.02em] uppercase px-[10px] py-[5px] rounded-[8px] border bg-raise select-none",
              pres.tone === 'neutral' && "border-line-2 text-ghost",
              pres.tone === 'green' && "border-[color-mix(in_oklab,var(--green)_40%,var(--line))] text-green",
              pres.tone === 'red' && "border-[color-mix(in_oklab,var(--red)_45%,var(--line))] text-red",
              pres.tone === 'gold' && "border-[color-mix(in_oklab,var(--gold)_40%,var(--line))] text-gold",
              pres.tone === 'amber' && "border-[color-mix(in_oklab,var(--amber)_45%,var(--line))] text-amber"
            )}>
              <span className={cn(
                "w-1.5 h-1.5 rounded-full",
                pres.tone === 'neutral' && "bg-ghost",
                pres.tone === 'green' && "bg-green",
                pres.tone === 'red' && "bg-red",
                pres.tone === 'gold' && "bg-gold",
                pres.tone === 'amber' && "bg-amber"
              )} />
              <span>{pres.label}</span>
            </div>
          ) : (
            <div className="inline-flex items-center gap-[7px] font-ui font-semibold text-[11.5px] tracking-[0.02em] uppercase px-[10px] py-[5px] rounded-[8px] border border-line-2 text-dim bg-raise select-none">
              <span className={cn(
                "w-1.5 h-1.5 rounded-full",
                state === 'merged' && "bg-green",
                (state === 'blocked' || state === 'impl-failed') && "bg-red",
                (state === 'reviewing' || state === 'fixing') && "bg-gold",
                state === 'unknown' && "bg-ghost",
                state !== 'merged' && state !== 'blocked' && state !== 'impl-failed' && state !== 'reviewing' && state !== 'fixing' && state !== 'unknown' && "bg-faint"
              )} />
              <span>{state}</span>
            </div>
          )}

          {isHosted ? (
            <span className="font-mono text-ghost text-[11.5px]">hosted goal status · not a GitHub marker</span>
          ) : (
            <span className="font-mono text-ghost text-[11.5px]">= max trusted <b className="text-faint font-medium">state:v1</b> · labels are hints</span>
          )}

          {isHosted ? (
            <>
              <span className="font-mono text-ghost text-[11.5px]">repo <b className="text-faint font-medium">{repoStr || '—'}</b></span>
              <span className="font-mono text-ghost text-[11.5px]">packages <b className="text-faint font-medium">{packageNames.length > 0 ? packageNames.join(', ') : '—'}</b></span>
              <span className="font-mono text-ghost text-[11.5px]">
                active session{' '}
                <b className="text-faint font-medium">
                  {activeSessionId ? (
                    <Link to="/settings" className="underline hover:text-dim">
                      {activeSessionId}
                    </Link>
                  ) : (
                    '—'
                  )}
                </b>
              </span>
            </>
          ) : (
            <>
              <span className="font-mono text-ghost text-[11.5px]">version <b className="text-faint font-medium">{version}</b></span>
              <span className="font-mono text-ghost text-[11.5px]">head <b className="text-faint font-medium">{headSha}</b></span>
              <span className="font-mono text-ghost text-[11.5px]">branch <b className="text-faint font-medium">{branch}</b></span>
              <span className="font-mono text-ghost text-[11.5px]">
                blocks <b className="text-faint font-medium">{blocksGoalId ? `#${blocksGoalId}` : '—'}</b>
              </span>
            </>
          )}
        </div>

        {/* Inert Decide Box if populated, or omitted by default */}
        {isReal && (
          <div className="mt-[18px] flex items-center gap-[18px] bg-raise border border-line border-l-[3px] border-l-red rounded-[12px] p-[15px_18px] max-[780px]:flex-wrap">
            <span className="font-ui font-semibold text-[11px] tracking-[0.06em] uppercase text-red flex-shrink-0">
              Real · autonomous
            </span>
            <div className="flex-1 min-w-0">
              <div className="font-semibold text-[15px] text-fg">
                Merges PR into the <span className="font-mono">integration</span> branch — autonomously
              </div>
              <div className="font-mono text-[11.5px] text-ghost mt-[3px] leading-relaxed">
                REAL posture · CI green · review approved · head-bound. No per-goal pause exists.
              </div>
            </div>
            <button disabled className="font-ui font-medium text-[13px] border border-line-2 bg-raise-2 text-dim rounded-control px-4 py-[9px] cursor-not-allowed opacity-50 flex-shrink-0">
              Global write → DRY-RUN
            </button>
          </div>
        )}
      </div>

      {/* Grid Layout */}
      <div className="grid grid-cols-[1fr_348px] max-[980px]:grid-cols-1 gap-[34px] items-start pb-10">
        {/* Left Side: Lifecycle Timeline */}
        <div className="min-w-0">
          <div className="font-mono text-eyebrow text-ghost uppercase mb-4">
            {isHosted ? (
              <span>Lifecycle · hosted goal status · not a GitHub marker</span>
            ) : (
              <span>Lifecycle · current state = MAX trusted state:v1 marker by (version, stage_rank) · fkst-dev labels are hints</span>
            )}
          </div>

          {hasData && lifecycleEvents.length > 0 ? (
            <div className="tl relative mt-4 pl-[26px] before:content-[''] before:absolute before:left-1.5 before:top-1 before:bottom-2 before:w-[1px] before:bg-line-2">
              {lifecycleEvents.map((ev, idx) => (
                <div key={idx} className={cn("ev relative mb-5", ev.isCurrent ? "font-semibold" : "")}>
                  <span className={cn(
                    "node absolute left-[-26px] top-[3px] w-[11px] h-[11px] rounded-full border-2 border-bg",
                    ev.isCurrent ? "bg-amber" : "bg-line-2",
                    ev.type === 'approve' && "bg-green",
                    ev.type === 'converge' && "bg-gold"
                  )} />
                  <div className="hd flex items-center justify-between gap-[9px]">
                    <div className="flex items-center gap-2">
                      <span className="nm font-semibold text-[13.5px] text-fg">{ev.name}</span>
                      {ev.type && (
                        <span className={cn(
                          "chip font-mono text-[10px] px-1.5 py-0.5 rounded-[5px] border",
                          ev.type === 'approve' && "text-green border-[color-mix(in_oklab,var(--green)_40%,var(--line))]",
                          ev.type === 'converge' && "text-gold border-[color-mix(in_oklab,var(--gold)_40%,var(--line))]"
                        )}>
                          {ev.type}
                        </span>
                      )}
                      {ev.isCurrent && (
                        <span className="now font-mono text-[9.5px] text-red border border-[color-mix(in_oklab,var(--red)_45%,var(--line))] rounded-[5px] px-1.5 py-0.5">
                          ● NOW{isReal ? ' · REAL' : ''}
                        </span>
                      )}
                    </div>
                    {ev.timestamp && <span className="ts font-mono text-[11px] text-ghost">{ev.timestamp}</span>}
                  </div>
                  {ev.body && <div className="body font-mono text-[11.5px] text-dim mt-1">{ev.body}</div>}
                  {ev.marker && (
                    <div className="marker font-mono text-[10.5px] text-ghost bg-bg border border-line rounded-[7px] p-[8px_10px] mt-2 break-all select-all">
                      {ev.marker}
                    </div>
                  )}
                  {ev.trustedBy && (
                    <div className="trust font-mono text-[10px] text-faint mt-1.5">
                      <span className="text-green">✓ {ev.trustedBy}</span> · trusted (matches <span className="font-mono text-[11px]">FKST_GITHUB_BOT_LOGIN</span>)
                    </div>
                  )}
                </div>
              ))}
            </div>
          ) : isHosted ? (
            <div className="flex items-center justify-center py-12 text-ghost font-mono text-[12px] border border-dashed border-line rounded-panel bg-raise/20">
              lifecycle timeline not exposed by the v1 API for hosted goals
            </div>
          ) : (
            <div className="flex items-center justify-center py-12 text-ghost font-mono text-[12px] border border-dashed border-line rounded-panel bg-raise/20">
              no GitHub plane connected — sign-in pending
            </div>
          )}
        </div>

        {/* Right Side: Side Panels (Diagnostics & Merge Gate) */}
        <div className="flex flex-col gap-4">
          {/* Merge Gate Panel */}
          <div className="panel bg-raise border border-line rounded-[13px] overflow-hidden">
            <div className="ph p-[11px_14px] border-b border-line font-mono font-semibold text-[11px] tracking-[0.1em] uppercase text-faint flex items-center justify-between">
              Merge gate
              {isHosted ? (
                <span className="text-ghost text-[10px] lowercase">unavailable</span>
              ) : mergeGate ? (
                <span className={cn(
                  "text-[10.5px] font-mono lowercase",
                  Object.values(mergeGate).every(v => v === 'ok')
                    ? "text-green"
                    : mergeGate.posture === 'unknown'
                      ? "text-ghost"
                      : "text-ghost"
                )}>
                  {Object.values(mergeGate).every(v => v === 'ok')
                    ? 'passing'
                    : mergeGate.posture === 'unknown'
                      ? 'unknown'
                      : 'blocked'}
                </span>
              ) : (
                <span className="text-ghost text-[10px] lowercase">not at the gate yet</span>
              )}
            </div>
            <div className="pc p-[13px_14px] flex flex-col gap-[11px]">
              {isHosted ? (
                <div className="flex flex-col items-center justify-center text-center py-6 text-ghost font-mono text-[11px]">
                  merge gate not exposed by the v1 API
                </div>
              ) : mergeGate ? (
                <>
                  <div className="gate flex items-center gap-[10px] text-[12.5px] text-dim">
                    {mergeGate.reviewApproved === 'ok' ? (
                      <span className="mk ok text-green font-mono w-3.5 text-center">✓</span>
                    ) : mergeGate.reviewApproved === 'fail' ? (
                      <span className="mk fail text-red font-mono w-3.5 text-center">✗</span>
                    ) : (
                      <span className="mk text-ghost font-mono w-3.5 text-center">—</span>
                    )}
                    <span>trusted <span className="font-mono text-[11px] text-dim">review-result:v1 approve</span></span>
                  </div>
                  <div className="gate flex items-center gap-[10px] text-[12.5px] text-dim">
                    {mergeGate.headBound === 'ok' ? (
                      <span className="mk ok text-green font-mono w-3.5 text-center">✓</span>
                    ) : mergeGate.headBound === 'fail' ? (
                      <span className="mk fail text-red font-mono w-3.5 text-center">✗</span>
                    ) : (
                      <span className="mk text-ghost font-mono w-3.5 text-center">—</span>
                    )}
                    <span>head-bound <span className="font-mono text-[11px] text-dim">merge-ready:v1</span></span>
                  </div>
                  <div className="gate flex items-center gap-[10px] text-[12.5px] text-dim">
                    {mergeGate.ciGreen === 'ok' ? (
                      <span className="mk ok text-green font-mono w-3.5 text-center">✓</span>
                    ) : mergeGate.ciGreen === 'fail' ? (
                      <span className="mk fail text-red font-mono w-3.5 text-center">✗</span>
                    ) : (
                      <span className="mk text-ghost font-mono w-3.5 text-center">—</span>
                    )}
                    <span>CI green <span className="font-mono text-[11px] text-ghost">statusCheckRollup</span></span>
                  </div>
                  <div className="gate flex items-center gap-[10px] text-[12.5px] text-dim">
                    {mergeGate.mergeable === 'ok' ? (
                      <span className="mk ok text-green font-mono w-3.5 text-center">✓</span>
                    ) : mergeGate.mergeable === 'fail' ? (
                      <span className="mk fail text-red font-mono w-3.5 text-center">✗</span>
                    ) : (
                      <span className="mk text-ghost font-mono w-3.5 text-center">—</span>
                    )}
                    <span>mergeable · same-repo head</span>
                  </div>
                  <div className="gate flex items-center gap-[10px] text-[12.5px] text-dim">
                    {mergeGate.posture === 'ok' ? (
                      <span className="mk ok text-green font-mono w-3.5 text-center">✓</span>
                    ) : mergeGate.posture === 'fail' ? (
                      <span className="mk fail text-red font-mono w-3.5 text-center">✗</span>
                    ) : (
                      <span className="mk text-ghost font-mono w-3.5 text-center">—</span>
                    )}
                    <span className="flex items-center gap-1.5">
                      FKST_GITHUB_WRITE = <PostureChip />
                    </span>
                  </div>
                </>
              ) : (
                <>
                  <div className="gate flex items-center gap-[10px] text-[12.5px] text-dim select-none opacity-50">
                    <span className="mk text-ghost font-mono w-3.5 text-center">—</span>
                    <span>CI status unknown</span>
                  </div>
                  <div className="gate flex items-center gap-[10px] text-[12.5px] text-dim select-none opacity-50">
                    <span className="mk text-ghost font-mono w-3.5 text-center">—</span>
                    <span>Review approval pending</span>
                  </div>
                  <div className="gate flex items-center gap-[10px] text-[12.5px] text-dim select-none font-mono">
                    <span className="text-ghost w-3.5 text-center">—</span>
                    <span className="flex items-center gap-1.5">
                      Posture: <PostureChip />
                    </span>
                  </div>
                </>
              )}
            </div>
          </div>

          {/* Deliveries · redb */}
          <div className={cn(
            "panel bg-raise border border-line rounded-[13px] overflow-hidden",
            deliveries.length === 0 && "opacity-50"
          )}>
            <div className="ph p-[11px_14px] border-b border-line font-mono font-semibold text-[11px] tracking-[0.1em] uppercase text-faint flex items-center justify-between">
              Deliveries · redb
            </div>
            <div className="pc p-[13px_14px] flex flex-col gap-[11px]">
              {deliveries.length > 0 ? (
                deliveries.map((d, i) => (
                  <div key={i} className="flex flex-col gap-1 text-dim">
                    <div className="drow flex items-center gap-[9px] font-mono text-[11px]">
                      <span className={cn(
                        "st font-mono text-[9.5px] px-[7px] py-[2px] rounded-[5px] border",
                        d.status === 'ACK' ? "text-green border-[color-mix(in_oklab,var(--green)_40%,var(--line))]" : "text-amber border-[color-mix(in_oklab,var(--amber)_40%,var(--line))]"
                      )}>
                        {d.status}
                      </span>
                      <span>{d.name} · gen {d.gen} · {d.state}</span>
                    </div>
                    {d.timeLeft && <div className="diag font-mono text-[10px] text-ghost pl-[48px]">timeLeft: {d.timeLeft}</div>}
                    {d.sourceRef && <div className="diag font-mono text-[10px] text-ghost pl-[48px]">{d.sourceRef}</div>}
                  </div>
                ))
              ) : (
                <div className="flex flex-col items-center justify-center text-center py-6 text-ghost font-mono text-[11px]">
                  host telemetry not connected
                </div>
              )}
            </div>
          </div>

          {/* Runs · codex */}
          <div className={cn(
            "panel bg-raise border border-line rounded-[13px] overflow-hidden",
            runs.length === 0 && "opacity-50"
          )}>
            <div className="ph p-[11px_14px] border-b border-line font-mono font-semibold text-[11px] tracking-[0.1em] uppercase text-faint flex items-center justify-between">
              Runs · codex <span className="text-[10px] lowercase text-ghost font-normal ml-2">diagnostic</span>
            </div>
            <div className="pc p-[13px_14px] flex flex-col gap-[11px]">
              {runs.length > 0 ? (
                runs.map((r, i) => (
                  <div key={i} className="run flex items-center gap-[9px] font-mono text-[11px] text-dim">
                    <span className={r.exitCode === 0 ? "e text-green" : "text-red"}>
                      exit {r.exitCode === null ? '—' : r.exitCode}
                    </span>
                    <span>
                      {r.action} · {r.duration}
                      {r.permits && ` · permits ${r.permits}`}
                    </span>
                  </div>
                ))
              ) : (
                <div className="flex flex-col items-center justify-center text-center py-6 text-ghost font-mono text-[11px]">
                  host telemetry not connected
                </div>
              )}
            </div>
          </div>

          {/* Consensus · review */}
          <div className={cn(
            "panel bg-raise border border-line rounded-[13px] overflow-hidden",
            !consensus && "opacity-50"
          )}>
            <div className="ph p-[11px_14px] border-b border-line font-mono font-semibold text-[11px] tracking-[0.1em] uppercase text-faint flex items-center justify-between">
              Review · consensus <span className="text-[10px] lowercase text-ghost font-normal ml-2">diagnostic</span>
            </div>
            <div className="pc p-[13px_14px] flex flex-col gap-[11px]">
              {consensus ? (
                <div className="flex flex-col gap-2">
                  <div className="flex items-center gap-[9px] text-[12.5px] text-dim">
                    {consensus.passes ? (
                      <span className="text-green font-mono w-3.5 text-center">✓</span>
                    ) : (
                      <span className="text-red font-mono w-3.5 text-center">✗</span>
                    )}
                    <span className="font-semibold text-fg">consensus {consensus.passes ? 'passed' : 'failed'}</span>
                  </div>
                  <div className="font-mono text-[11px] text-ghost leading-relaxed">
                    {consensus.summary}
                  </div>
                </div>
              ) : (
                <div className="flex flex-col items-center justify-center text-center py-6 text-ghost font-mono text-[11px]">
                  host telemetry not connected
                </div>
              )}
            </div>
          </div>
        </div>
      </div>

      {/* Footer */}
      <div className="foot flex gap-6 font-mono text-[11px] text-ghost flex-wrap mt-6 pt-[14px] border-t border-line">
        <span>state as of <b>unknown</b> · merge gate from redb <b>unknown</b> · poll-derived</span>
        {isHosted ? (
          <span>hosted goal record; not GitHub-derived</span>
        ) : (
          <span>every transition re-derived from GitHub on each tick</span>
        )}
      </div>
    </div>
  );
}
