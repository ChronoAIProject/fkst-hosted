// Boundary validation for both operations payloads.
//
// The rule this module enforces is narrow and deliberate: **every field a
// renderer dereferences must be proven present and of the right type before the
// data reaches React**, and a payload that fails is rejected WHOLE. There is no
// partial adoption, because a half-validated page is exactly the state in which
// a row of unknown provenance gets drawn.
//
// The scope check is the security-shaped half. The page asks for a scope and the
// server states the scope it served. If those disagree — we asked for `mine` and
// were handed `all` — we cannot say whose rows these are, so the payload is a
// hard failure (`scope_mismatch`) and NOTHING is rendered. A caller that stated
// no scope (the very first load, where the server owns the default) accepts
// whichever scope comes back and adopts it.

import { OperationsError } from './errors';
import type {
  ActivityPage,
  ActivityRow,
  ActivityScope,
  SandboxInventory,
  SandboxRow,
  SandboxScope,
} from './types';
import { ACTIVITY_SCOPES, SANDBOX_SCOPES } from './types';

/** Reject the whole payload. */
function malformed(): never {
  throw new OperationsError('malformed', 200);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function str(value: unknown): value is string {
  return typeof value === 'string';
}

/** `null`, `undefined`, or a string — the shape every optional wire string has. */
function optionalStr(value: unknown): boolean {
  return value == null || typeof value === 'string';
}

/** `null`, `undefined`, or a finite number. `NaN`/`Infinity` are rejected: they
 *  would render as "NaN" in a duration or age cell. */
function optionalNum(value: unknown): boolean {
  return value == null || (typeof value === 'number' && Number.isFinite(value));
}

function num(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

function strArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every(str);
}

/** An identity block: every member optional, but never of a surprising type. */
function identity(value: unknown): boolean {
  if (!isRecord(value)) return false;
  return optionalStr(value.kind) && optionalNum(value.id) && optionalStr(value.login);
}

function correlation(value: unknown): boolean {
  if (!isRecord(value)) return false;
  return (
    optionalStr(value.session_id) &&
    optionalStr(value.repo_full_name) &&
    optionalNum(value.installation_id) &&
    optionalNum(value.trigger_issue) &&
    optionalStr(value.request_id) &&
    optionalStr(value.webhook_delivery_id)
  );
}

/** Validate one activity row against its own discriminated contract. A lifecycle
 *  row is NOT allowed to carry a method/status, and an api_request row is not
 *  allowed to carry a lifecycle action — mixing them is how fake HTTP values
 *  would end up in a lifecycle cell. */
function activityRow(value: unknown): value is ActivityRow {
  if (!isRecord(value)) return false;
  if (!str(value.event_id) || !str(value.delivery_state) || !str(value.source)) return false;
  if (!identity(value.actor) || !isRecord(value.principal)) return false;
  if (!optionalStr((value.principal as Record<string, unknown>).kind)) return false;
  if (!optionalStr((value.principal as Record<string, unknown>).id)) return false;
  if (!correlation(value.correlation)) return false;

  if (value.record_kind === 'api_request') {
    return (
      str(value.completed_at) &&
      str(value.method) &&
      str(value.route_template) &&
      str(value.operation_id) &&
      str(value.outcome) &&
      isRecord(value.arguments) &&
      optionalStr(value.request_id) &&
      optionalStr(value.started_at) &&
      optionalStr(value.arguments_parse_status) &&
      optionalStr(value.error_code) &&
      optionalNum(value.status_code) &&
      optionalNum(value.duration_ms)
    );
  }
  if (value.record_kind === 'sandbox_lifecycle') {
    return (
      str(value.occurred_at) &&
      str(value.lifecycle_action) &&
      str(value.session_id) &&
      identity(value.creator) &&
      identity(value.trigger_author) &&
      optionalStr(value.backend) &&
      optionalStr(value.runtime_id) &&
      optionalStr(value.created_at) &&
      optionalStr(value.reason_code)
    );
  }
  return false;
}

function sourceStatus(value: unknown): boolean {
  if (!isRecord(value)) return false;
  return (
    str(value.posthog) &&
    str(value.relay) &&
    typeof value.partial === 'boolean' &&
    optionalStr(value.message_code)
  );
}

/**
 * Validate one activity page, then prove it answers the scope that was asked
 * for. `requested` is `null` only on a first load that deliberately let the
 * server choose.
 */
export function validateActivityPage(body: unknown, requested: ActivityScope | null): ActivityPage {
  if (!isRecord(body)) malformed();
  if (!str(body.queried_at) || !str(body.from) || !str(body.to)) malformed();
  if (typeof body.can_view_all !== 'boolean') malformed();
  if (!optionalStr(body.next_cursor)) malformed();
  if (!sourceStatus(body.source_status)) malformed();
  const scope = body.effective_scope;
  if (!str(scope) || !(ACTIVITY_SCOPES as readonly string[]).includes(scope)) malformed();
  if (!Array.isArray(body.items) || !body.items.every(activityRow)) malformed();
  // The security gate: an answered scope that is not the asked-for scope means
  // we do not know whose rows these are.
  if (requested !== null && scope !== requested) {
    throw new OperationsError('scope_mismatch', 200);
  }
  // A regular caller may never be handed the global scope, whatever they asked.
  if (scope === 'all' && body.can_view_all !== true) {
    throw new OperationsError('scope_mismatch', 200);
  }
  return body as unknown as ActivityPage;
}

function sandboxRow(value: unknown): value is SandboxRow {
  if (!isRecord(value)) return false;
  return (
    str(value.backend) &&
    str(value.runtime_id) &&
    str(value.metadata_state) &&
    str(value.attribution_source) &&
    str(value.status) &&
    str(value.raw_status) &&
    typeof value.managed === 'boolean' &&
    num(value.minimum_lifetime_seconds) &&
    num(value.idle_grace_seconds) &&
    strArray(value.warning_codes) &&
    optionalStr(value.runtime_name) &&
    optionalStr(value.runtime_uid) &&
    optionalStr(value.backend_location) &&
    optionalStr(value.session_id) &&
    optionalStr(value.creator_login) &&
    optionalStr(value.trigger_author_login) &&
    optionalStr(value.repo_full_name) &&
    optionalStr(value.status_reason) &&
    optionalStr(value.status_message) &&
    optionalStr(value.created_at) &&
    optionalStr(value.expires_at) &&
    optionalStr(value.last_pending_at) &&
    optionalStr(value.last_transition_at) &&
    optionalStr(value.deletion_timestamp) &&
    optionalNum(value.creator_id) &&
    optionalNum(value.trigger_author_id) &&
    optionalNum(value.installation_id) &&
    optionalNum(value.trigger_issue) &&
    optionalNum(value.age_seconds) &&
    optionalNum(value.max_lifetime_seconds) &&
    optionalNum(value.remaining_seconds) &&
    optionalNum(value.minimum_lifetime_remaining_seconds) &&
    optionalNum(value.idle_for_seconds) &&
    optionalNum(value.restart_count)
  );
}

/** Validate one live snapshot, then prove it answers the requested scope. */
export function validateSandboxInventory(
  body: unknown,
  requested: SandboxScope | null
): SandboxInventory {
  if (!isRecord(body)) malformed();
  if (!str(body.observed_at) || !str(body.backend)) malformed();
  if (typeof body.can_view_all !== 'boolean') malformed();
  if (!num(body.item_count)) malformed();
  if (!isRecord(body.filters_applied)) malformed();
  if (!strArray(body.warning_codes)) malformed();
  const scope = body.effective_scope;
  if (!str(scope) || !(SANDBOX_SCOPES as readonly string[]).includes(scope)) malformed();
  if (!Array.isArray(body.items) || !body.items.every(sandboxRow)) malformed();
  // `item_count` is documented as the length of `items`; a disagreement means
  // one of the two was derived from rows this caller cannot see.
  if (body.item_count !== body.items.length) malformed();
  if (requested !== null && scope !== requested) {
    throw new OperationsError('scope_mismatch', 200);
  }
  if (scope === 'all' && body.can_view_all !== true) {
    throw new OperationsError('scope_mismatch', 200);
  }
  return body as unknown as SandboxInventory;
}
