/**
 * Executing a confirmed proposal.
 *
 * The security boundary lives here, and it is a whitelist: each `kind` maps to ONE
 * typed API function — the same one the dashboard's own button calls — and there is
 * deliberately no generic method/path executor. `proposal.target` is display copy for
 * the card footer; nothing in this module reads it. A hijacked model can therefore
 * change what a card SAYS, never which endpoint a confirmation reaches.
 *
 * Extracted from the provider because it is the part worth testing in isolation: it is
 * pure request-mapping plus outcome-shaping, with no React in it.
 */

import {
  createRepo,
  createWorkItem,
  createTrigger,
  stopTrigger,
  uninstallApp,
  type ApiFetch,
} from '@/lib/api/canvas';
import { deleteEnvironmentProfile, putEnvironmentProfile } from '@/lib/api/environments';
import { mapDraftToRequest } from './action-types';
import type { ActionProposal } from './action-types';

/** Extra input a card collects that the proposal itself must never carry.
 *
 *  Today that is exactly one thing: environment secret VALUES, which the user types on
 *  the card. They are passed straight to the API call and are never stored in the
 *  transcript, in `sessionStorage`, or in the proposal — the whole reason the model only
 *  ever drafts secret NAMES. */
export interface ProposalExecutionInput {
  secrets?: Record<string, string>;
}

/** What executing a proposal produced, in the shape the transcript records. */
export interface ProposalOutcome {
  ok: boolean;
  /** Server-supplied failure text, already human-readable. */
  message?: string;
  /** The created issue, when the action created one. */
  issueNumber?: number;
  issueUrl?: string;
}

/** Render a failed install validation as something a user can act on.
 *
 *  The bare envelope message ("install validation failed") is useless on its own; the
 *  command that failed and the tail of its stderr are the whole diagnosis, and the
 *  backend already redacts the tail. */
function validationMessage(validation: {
  message: string;
  failed_command: string;
  exit_code: number;
  timed_out: boolean;
  stderr_tail: string;
}): string {
  const parts = [validation.message];
  if (validation.timed_out) {
    parts.push('The install commands exceeded the validation deadline.');
  } else if (validation.failed_command !== '') {
    parts.push(`Failed command: ${validation.failed_command} (exit ${validation.exit_code})`);
  }
  if (validation.stderr_tail.trim() !== '') parts.push(validation.stderr_tail.trim());
  return parts.join('\n');
}

/**
 * Adapt a proposal run to the `MutationResult` shape `ConfirmDialog` takes.
 *
 * The dialog owns the mutation for the destructive kinds (its verified contract), and it
 * speaks the API layer's result type — so the adapter lives here next to the mapping it
 * adapts, rather than as a lambda inside the card.
 */
export function runProposalAsMutation(
  apiFetch: ApiFetch,
  proposal: ActionProposal,
  input: ProposalExecutionInput = {}
): Promise<{ ok: true; data: null } | { ok: false; message: string | null }> {
  return runProposal(apiFetch, proposal, input).then((outcome) =>
    outcome.ok
      ? ({ ok: true, data: null } as const)
      : ({ ok: false, message: outcome.message ?? null } as const)
  );
}

/**
 * Run one confirmed proposal.
 *
 * Never throws: a rejected mutation is an outcome the card renders, not an exception the
 * provider has to catch in two places.
 */
export async function runProposal(
  apiFetch: ApiFetch,
  proposal: ActionProposal,
  input: ProposalExecutionInput = {}
): Promise<ProposalOutcome> {
  switch (proposal.kind) {
    case 'create_session': {
      const result = await createTrigger(
        apiFetch,
        proposal.owner,
        proposal.name,
        mapDraftToRequest(proposal.request)
      );
      return result.ok
        ? { ok: true, issueNumber: result.data.issue_number, issueUrl: result.data.html_url }
        : { ok: false, message: result.message ?? undefined };
    }

    case 'create_work_item': {
      const result = await createWorkItem(
        apiFetch,
        proposal.owner,
        proposal.name,
        proposal.trigger_issue_number,
        {
          title: proposal.title,
          ...(proposal.label ? { label: proposal.label } : {}),
          body: proposal.body,
        }
      );
      return result.ok
        ? { ok: true, issueNumber: result.data.issue_number, issueUrl: result.data.html_url }
        : { ok: false, message: result.message ?? undefined };
    }

    case 'stop_session': {
      const result = await stopTrigger(
        apiFetch,
        proposal.owner,
        proposal.name,
        proposal.trigger_issue_number
      );
      return result.ok ? { ok: true } : { ok: false, message: result.message ?? undefined };
    }

    case 'create_repository': {
      const result = await createRepo(apiFetch, {
        ...(proposal.owner ? { owner: proposal.owner } : {}),
        name: proposal.name,
        private: proposal.private,
        ...(proposal.description ? { description: proposal.description } : {}),
      });
      return result.ok ? { ok: true } : { ok: false, message: result.message ?? undefined };
    }

    case 'save_environment_profile': {
      const result = await putEnvironmentProfile(apiFetch, proposal.profile_name, {
        install: proposal.install,
        variables: Object.fromEntries(proposal.variables.map((entry) => [entry.key, entry.value])),
        // The values never came from the model: the card collected them a moment ago.
        secrets: input.secrets ?? {},
      });
      if (result.ok) return { ok: true };
      // The 422 install-validation report is a DIFFERENT failure shape from the plain
      // envelope, and it is the only one that says which command failed — so it is
      // narrowed explicitly rather than folded into a generic message.
      return 'validation' in result
        ? { ok: false, message: validationMessage(result.validation) }
        : { ok: false, message: result.message ?? undefined };
    }

    case 'delete_environment_profile': {
      const result = await deleteEnvironmentProfile(apiFetch, proposal.profile_name);
      return result.ok ? { ok: true } : { ok: false, message: result.message ?? undefined };
    }

    case 'uninstall_app': {
      const result = await uninstallApp(apiFetch, proposal.owner);
      return result.ok ? { ok: true } : { ok: false, message: result.message ?? undefined };
    }
  }
}
