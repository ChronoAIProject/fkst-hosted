import type { CreateSessionRequest } from '@/lib/api/types';

/**
 * The wire `ActionProposal` union, mirroring the backend's typed schema.
 *
 * These values arrive from the model's turn, so they are DATA — never anything
 * evaluated, and `target` is display-only. The SPA maps `kind` to its own typed API
 * function; blindly fetching `target.path` would hand the model back the write
 * capability the whole design removes.
 */

/** The create-session subset a chat draft may carry — the backend's
 *  `DraftSessionRequest`, which has no field for secrets by construction. */
export interface DraftSessionRequest {
  name: string;
  packages: string[];
  manifests: string[];
  work_label?: string | null;
  environment?: string | null;
  source_branch?: string | null;
  target_branch?: string | null;
  auto_merge?: boolean | null;
  log_access: string[];
  collaborators: string[];
  output_lang?: string | null;
}

/** Descriptive endpoint metadata for the card footer. Display only. */
export interface ActionTarget {
  method: string;
  path: string;
}

export interface CreateSessionProposal {
  kind: 'create_session';
  owner: string;
  name: string;
  request: DraftSessionRequest;
  /** The exact issue body a confirmation will file. */
  rendered_issue_body: string;
  summary: string;
  target: ActionTarget;
}

export interface CreateWorkItemProposal {
  kind: 'create_work_item';
  owner: string;
  name: string;
  trigger_issue_number: number;
  title: string;
  label?: string | null;
  body: string;
  summary: string;
  target: ActionTarget;
}

export interface StopSessionProposal {
  kind: 'stop_session';
  owner: string;
  name: string;
  trigger_issue_number: number;
  reason: string;
  summary: string;
  target: ActionTarget;
}

export type ActionProposal = CreateSessionProposal | CreateWorkItemProposal | StopSessionProposal;

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value != null;

const isString = (value: unknown): value is string => typeof value === 'string';

/** A non-empty string. Used where an empty value would render a blank card. */
const isText = (value: unknown): value is string => isString(value) && value.trim().length > 0;

/** An optional string field: absent, null, or a string. */
const isOptionalString = (value: unknown): boolean =>
  value === undefined || value === null || isString(value);

const isStringArray = (value: unknown): value is string[] =>
  Array.isArray(value) && value.every(isString);

const isTarget = (value: unknown): value is ActionTarget =>
  isRecord(value) && isString(value.method) && isString(value.path);

/** A positive integer issue number. */
const isIssueNumber = (value: unknown): value is number =>
  typeof value === 'number' && Number.isInteger(value) && value > 0;

function isDraft(value: unknown): value is DraftSessionRequest {
  if (!isRecord(value)) return false;
  return (
    isText(value.name) &&
    isStringArray(value.packages) &&
    isStringArray(value.manifests) &&
    isStringArray(value.log_access) &&
    isStringArray(value.collaborators) &&
    isOptionalString(value.work_label) &&
    isOptionalString(value.environment) &&
    isOptionalString(value.source_branch) &&
    isOptionalString(value.target_branch) &&
    isOptionalString(value.output_lang) &&
    (value.auto_merge === undefined ||
      value.auto_merge === null ||
      typeof value.auto_merge === 'boolean')
  );
}

/**
 * Structurally validate a proposal from the stream.
 *
 * Returns `null` for anything malformed OR for an unrecognized `kind`. There is
 * deliberately no "unsupported action" card: the union has exactly three variants,
 * and a card the SPA cannot execute is worse than an honest note saying the draft
 * was unreadable.
 */
export function parseActionProposal(value: unknown): ActionProposal | null {
  if (!isRecord(value)) return null;
  if (!isText(value.owner) || !isText(value.name)) return null;
  if (!isText(value.summary) || !isTarget(value.target)) return null;

  switch (value.kind) {
    case 'create_session':
      if (!isDraft(value.request) || !isString(value.rendered_issue_body)) return null;
      return value as unknown as CreateSessionProposal;

    case 'create_work_item':
      if (!isIssueNumber(value.trigger_issue_number)) return null;
      if (!isText(value.title) || !isString(value.body)) return null;
      if (!isOptionalString(value.label)) return null;
      return value as unknown as CreateWorkItemProposal;

    case 'stop_session':
      if (!isIssueNumber(value.trigger_issue_number)) return null;
      if (!isText(value.reason)) return null;
      return value as unknown as StopSessionProposal;

    default:
      return null;
  }
}

/**
 * Map a draft onto the real create-session request body.
 *
 * `disposable_environment` is never set: the draft has no field for it, which is
 * exactly why a dedicated DTO exists. Nulls become `undefined` so an omitted
 * section is omitted rather than sent as an explicit null.
 */
export function mapDraftToRequest(draft: DraftSessionRequest): CreateSessionRequest {
  return {
    name: draft.name,
    packages: draft.packages,
    manifests: draft.manifests,
    work_label: draft.work_label ?? undefined,
    environment: draft.environment ?? undefined,
    source_branch: draft.source_branch ?? undefined,
    target_branch: draft.target_branch ?? undefined,
    auto_merge: draft.auto_merge ?? undefined,
    log_access: draft.log_access,
    collaborators: draft.collaborators,
    output_lang: draft.output_lang ?? undefined,
  } as CreateSessionRequest;
}
