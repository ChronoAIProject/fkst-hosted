import { useState } from 'react';
import { Link } from 'react-router-dom';
import { Chip } from '@/components/ui/chip';
import { ErrorNote } from '@/components/ui/error-note';
import { MarkdownPreview } from '@/components/ui/markdown-preview';
import { Reveal } from '@/components/ui/motion';
import { ConfirmDialog } from '@/components/modals/confirm-dialog';
import { useContent } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';
import { stopTrigger } from '@/lib/api/canvas';
import { RESTORED_UNKNOWN, useChat } from './chat-context';
import type { ChatProposal } from './chat-context';
import type { ActionProposal } from './action-types';

/** The kind chip's label. */
function kindLabel(proposal: ActionProposal, s: ReturnType<typeof useContent>['chat']): string {
  switch (proposal.kind) {
    case 'create_session':
      return s.kindNewSession;
    case 'create_work_item':
      return s.kindWorkItem;
    case 'stop_session':
      return s.kindStopSession;
  }
}

/** The per-kind body. Each shows the thing that will actually be created, because
 *  the confirm gate is only meaningful if the user can see what they are approving. */
function CardBody({ proposal }: { proposal: ActionProposal }) {
  const s = useContent().chat;
  const [previewOpen, setPreviewOpen] = useState(false);

  if (proposal.kind === 'create_session') {
    const { request } = proposal;
    return (
      <div className="flex flex-col gap-2">
        {/* Collapsed by default: the exact issue body is the authoritative preview,
            but it is long, and the field table answers most questions at a glance. */}
        <button
          type="button"
          onClick={() => setPreviewOpen((open) => !open)}
          aria-expanded={previewOpen}
          data-testid="chat-action-preview-toggle"
          className="self-start rounded-control font-mono text-[10px] uppercase tracking-[0.12em] text-ghost transition-colors hover:text-faint cursor-pointer"
        >
          {s.previewToggle} {previewOpen ? '▴' : '▾'}
        </button>
        <Reveal open={previewOpen}>
          <pre
            data-testid="chat-action-preview"
            className="max-h-64 overflow-auto rounded-control border border-line bg-glass px-3 py-2 font-mono text-[11.5px] leading-5 text-dim whitespace-pre-wrap"
          >
            {proposal.rendered_issue_body}
          </pre>
        </Reveal>
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 font-mono text-[10.5px]">
          {[
            [s.fieldWorkLabel, request.work_label ?? s.fieldAutoDiscovered],
            [s.fieldPackages, String(request.packages.length + request.manifests.length)],
            [
              s.fieldBranches,
              `${request.source_branch ?? s.fieldDefault} → ${request.target_branch ?? 'fkst-hosted-default'}`,
            ],
            [s.fieldAutoMerge, request.auto_merge ? s.on : s.off],
            ...(request.environment ? [[s.fieldEnvironment, request.environment]] : []),
          ].map(([label, value]) => (
            <div key={label} className="contents">
              <dt className="text-ghost uppercase tracking-[0.1em]">{label}</dt>
              <dd className="text-faint">{value}</dd>
            </div>
          ))}
        </dl>
      </div>
    );
  }

  if (proposal.kind === 'create_work_item') {
    return (
      <div className="flex flex-col gap-2">
        <p className="font-ui text-[12.5px] font-semibold text-fg">{proposal.title}</p>
        {proposal.label && (
          <span className="font-mono text-[10.5px] text-faint">{proposal.label}</span>
        )}
        {proposal.body !== '' && (
          <MarkdownPreview markdown={proposal.body} ariaLabel={s.workItemBodyAria} />
        )}
      </div>
    );
  }

  return (
    // Stopping is destructive, so it is styled with --warn — never the brand accent.
    <div className="flex flex-col gap-1 border-l-2 border-l-warn pl-2.5">
      <p className="font-mono text-[11.5px] text-warn">
        {s.stopTriggerLine.replace('{number}', String(proposal.trigger_issue_number))}
      </p>
      <p className="text-[12px] leading-relaxed text-dim">{proposal.reason}</p>
    </div>
  );
}

/**
 * A confirm-gated proposal card.
 *
 * The human click IS the authorization boundary: the chat layer holds no write
 * capability, and confirming calls the same typed API function the dashboard's own
 * buttons use, under the user's own token. Nothing here ever auto-executes —
 * including on a transcript restore.
 */
export function ActionCard({ entry }: { entry: ChatProposal }) {
  const s = useContent().chat;
  const { apiFetch } = useAuth();
  const { executeProposal, dismissProposal, markProposalSucceeded } = useChat();
  const { proposal, state } = entry;
  const [confirmOpen, setConfirmOpen] = useState(false);

  const executing = state === 'executing';
  const succeeded = state === 'succeeded';
  // A restored mid-flight proposal is `failed` with a sentinel, because its real
  // outcome is unknowable — the copy says exactly that rather than pretending.
  const errorText = entry.error === RESTORED_UNKNOWN ? s.restoredUnknown : (entry.error ?? null);

  if (succeeded) {
    const dashboardHref =
      proposal.kind === 'create_session' && entry.issueNumber != null
        ? `/dashboard?owner=${encodeURIComponent(proposal.owner)}&repo=${encodeURIComponent(
            proposal.name
          )}&session=trigger-${entry.issueNumber}`
        : null;
    return (
      <div
        data-testid="chat-action-card"
        data-state="succeeded"
        className="rounded-card border border-line bg-raise px-2.5 py-2 flex items-center gap-2 flex-wrap"
      >
        <Chip tone="green">
          {proposal.kind === 'stop_session' ? s.outcomeChipStopped : s.outcomeChipCreated}
        </Chip>
        <span className="font-mono text-[10.5px] text-faint">
          {proposal.owner}/{proposal.name}
        </span>
        <span className="flex-1" aria-hidden="true" />
        {entry.issueUrl && (
          <a
            href={entry.issueUrl}
            target="_blank"
            rel="noopener noreferrer"
            data-testid="chat-action-issue-link"
            className="rounded-chip border border-line-2 bg-raise-2 px-2 py-0.5 font-mono text-[10px] uppercase tracking-[0.1em] text-faint no-underline transition-colors hover:text-fg"
          >
            {s.openIssue} ↗
          </a>
        )}
        {dashboardHref && (
          <Link
            to={dashboardHref}
            data-testid="chat-action-dashboard-link"
            className="rounded-chip border border-line-2 bg-raise-2 px-2 py-0.5 font-mono text-[10px] uppercase tracking-[0.1em] text-faint no-underline transition-colors hover:text-fg"
          >
            {s.openInDashboard}
          </Link>
        )}
      </div>
    );
  }

  return (
    <div
      data-testid="chat-action-card"
      data-state={state}
      className="rounded-card border border-line bg-glass backdrop-blur-glass p-2.5 flex flex-col gap-2 shadow-[var(--shadow-1),var(--highlight-top)]"
    >
      <div className="flex items-center gap-2 flex-wrap">
        <Chip tone="neutral">{kindLabel(proposal, s)}</Chip>
        <span className="font-mono text-[10.5px] text-ghost">
          {proposal.owner}/{proposal.name}
        </span>
      </div>
      <p className="text-[12.5px] leading-relaxed text-dim">{proposal.summary}</p>

      <CardBody proposal={proposal} />

      <p className="font-mono text-[10px] text-ghost">
        {proposal.target.method} {proposal.target.path}
      </p>
      {/* Said plainly, because the proposal validator deliberately does NOT check
          repository authority or label collisions — those run at confirmation. */}
      <p className="font-mono text-[10px] text-ghost">{s.finalChecksNote}</p>

      {/* The stop path routes through ConfirmDialog, which owns its own inline
          error — so this card shows none for that kind (its verified contract). */}
      {errorText != null && proposal.kind !== 'stop_session' && <ErrorNote message={errorText} />}

      <div className="flex items-center gap-2">
        <button
          type="button"
          disabled={executing}
          onClick={() => {
            if (proposal.kind === 'stop_session') {
              setConfirmOpen(true);
              return;
            }
            void executeProposal(entry.id);
          }}
          data-testid="chat-action-confirm"
          className="rounded-control bg-grad-accent px-3 py-1.5 font-ui text-[12px] font-semibold text-amber-ink shadow-[var(--shadow-1),var(--glow-amber)] transition-[filter,opacity] hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-50 cursor-pointer"
        >
          {executing ? (
            <>
              {s.executing}
              <span aria-hidden="true" className="anim-dot-blink ml-1 inline-block">
                ·
              </span>
            </>
          ) : (
            s.confirmExecute
          )}
        </button>
        <button
          type="button"
          disabled={executing}
          onClick={() => dismissProposal(entry.id)}
          data-testid="chat-action-dismiss"
          className="glass grad-border rounded-control px-3 py-1.5 font-ui text-[12px] font-semibold text-dim transition-colors hover:bg-raise-2 disabled:cursor-not-allowed disabled:opacity-50 cursor-pointer"
        >
          {s.dismiss}
        </button>
      </div>

      {confirmOpen && proposal.kind === 'stop_session' && (
        <ConfirmDialog
          title={s.stopConfirmTitle}
          body={s.stopConfirmBody
            .replace('{number}', String(proposal.trigger_issue_number))
            .replace('{repo}', `${proposal.owner}/${proposal.name}`)}
          confirmLabel={s.stopConfirmAction}
          pendingLabel={s.executing}
          cancelLabel={s.dismiss}
          // The dialog runs the mutation and owns its own error/close. The card's
          // own state is then updated through the same execute path on success, so
          // the success row renders exactly as it does for the other kinds.
          action={() =>
            stopTrigger(apiFetch, proposal.owner, proposal.name, proposal.trigger_issue_number)
          }
          fallbackError={s.executeFailed}
          onClose={() => setConfirmOpen(false)}
          onDone={() => {
            setConfirmOpen(false);
            // The dialog ALREADY ran the mutation (its verified contract), so this
            // only records the outcome — calling executeProposal here would close
            // the trigger a second time.
            markProposalSucceeded(entry.id);
          }}
        />
      )}
    </div>
  );
}

/** The cards under one assistant message. */
export function ActionCards({ proposals }: { proposals: ChatProposal[] }) {
  if (proposals.length === 0) return null;
  return (
    <div data-testid="chat-action-cards" className="flex flex-col gap-1.5">
      {proposals.map((entry) => (
        <ActionCard key={entry.id} entry={entry} />
      ))}
    </div>
  );
}
