/**
 * Per-kind presentation facts for a proposal card: what to call it, what it acts on,
 * whether it needs a destructive confirmation, and what to say once it succeeded.
 *
 * A lookup module rather than branches scattered through the card, because every one of
 * these is a per-kind decision and keeping them together is what makes "did we handle the
 * new kind everywhere?" answerable by reading one file. Each function switches
 * exhaustively over the union, so adding a kind fails the typecheck here first.
 */

import type { useContent } from '@/i18n';
import type { ActionProposal } from './action-types';

type ChatCopy = ReturnType<typeof useContent>['chat'];

/** The chip label naming the kind of action. */
export function kindLabel(proposal: ActionProposal, s: ChatCopy): string {
  switch (proposal.kind) {
    case 'create_session':
      return s.kindNewSession;
    case 'create_work_item':
      return s.kindWorkItem;
    case 'stop_session':
      return s.kindStopSession;
    case 'create_repository':
      return s.kindNewRepo;
    case 'save_environment_profile':
      return s.kindSaveEnv;
    case 'delete_environment_profile':
      return s.kindDeleteEnv;
    case 'uninstall_app':
      return s.kindUninstallApp;
  }
}

/**
 * What the action acts ON, as the card's small mono line.
 *
 * Not always a repository: an environment profile belongs to the user, and a repository
 * draft has no owner at all when it targets the personal account.
 */
export function scopeLine(proposal: ActionProposal, s: ChatCopy): string {
  switch (proposal.kind) {
    case 'create_session':
    case 'create_work_item':
    case 'stop_session':
      return `${proposal.owner}/${proposal.name}`;
    case 'create_repository':
      return `${proposal.owner ?? s.scopePersonal}/${proposal.name}`;
    case 'save_environment_profile':
    case 'delete_environment_profile':
      return s.scopeYourAccount;
    case 'uninstall_app':
      return proposal.owner;
  }
}

/** Copy for the modal a destructive action must pass through, or `null` when the kind
 *  executes on a single click.
 *
 *  The three destructive kinds are the ones with no undo: a retired session never
 *  revives, a deleted profile's secret values are gone, and an uninstall takes fkst off
 *  every repository of an account at once. */
export function destructiveConfirm(
  proposal: ActionProposal,
  s: ChatCopy
): { title: string; body: string; action: string } | null {
  switch (proposal.kind) {
    case 'stop_session':
      return {
        title: s.stopConfirmTitle,
        body: s.stopConfirmBody
          .replace('{number}', String(proposal.trigger_issue_number))
          .replace('{repo}', `${proposal.owner}/${proposal.name}`),
        action: s.stopConfirmAction,
      };
    case 'delete_environment_profile':
      return {
        title: s.deleteEnvConfirmTitle,
        body: s.deleteEnvConfirmBody.replace('{name}', proposal.profile_name),
        action: s.deleteEnvConfirmAction,
      };
    case 'uninstall_app':
      return {
        title: s.uninstallConfirmTitle,
        body: s.uninstallConfirmBody.replace('{owner}', proposal.owner),
        action: s.uninstallConfirmAction,
      };
    default:
      return null;
  }
}

/** The chip on the success row: what HAPPENED, in one word. */
export function outcomeChip(proposal: ActionProposal, s: ChatCopy): string {
  switch (proposal.kind) {
    case 'create_session':
    case 'create_work_item':
    case 'create_repository':
      return s.outcomeChipCreated;
    case 'stop_session':
      return s.outcomeChipStopped;
    case 'save_environment_profile':
      return s.outcomeChipSaved;
    case 'delete_environment_profile':
      return s.outcomeChipDeleted;
    case 'uninstall_app':
      return s.outcomeChipRemoved;
  }
}

/** The sentence recorded in the transcript once a confirmed proposal succeeded. */
export function outcomeNote(
  proposal: ActionProposal,
  s: ChatCopy,
  issueNumber?: number
): string {
  switch (proposal.kind) {
    case 'create_session':
      return s.outcomeSession
        .replace('{number}', String(issueNumber ?? 0))
        .replace('{repo}', `${proposal.owner}/${proposal.name}`);
    case 'create_work_item':
      return s.outcomeWorkItem
        .replace('{number}', String(issueNumber ?? 0))
        .replace('{repo}', `${proposal.owner}/${proposal.name}`);
    case 'stop_session':
      return s.outcomeStopped
        .replace('{number}', String(proposal.trigger_issue_number))
        .replace('{repo}', `${proposal.owner}/${proposal.name}`);
    case 'create_repository':
      return s.outcomeRepo.replace(
        '{repo}',
        `${proposal.owner ?? s.scopePersonal}/${proposal.name}`
      );
    case 'save_environment_profile':
      return s.outcomeEnvSaved.replace('{name}', proposal.profile_name);
    case 'delete_environment_profile':
      return s.outcomeEnvDeleted.replace('{name}', proposal.profile_name);
    case 'uninstall_app':
      return s.outcomeUninstalled.replace('{owner}', proposal.owner);
  }
}

/** Secret NAMES the card must collect a value for before it may be confirmed.
 *
 *  Only the environment save has any; every other kind returns an empty list, which is
 *  what keeps the confirm-gating rule in the card down to "are these all filled?". */
export function requiredSecretKeys(proposal: ActionProposal): string[] {
  return proposal.kind === 'save_environment_profile' ? proposal.secret_keys : [];
}
