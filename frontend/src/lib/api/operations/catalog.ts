// The operation ids the activity `operation_id` filter offers.
//
// This is a STATIC mirror of the deployment's audited OpenAPI catalog, not a
// fetched one: the backend validates `operation_id` against its own catalog and
// answers `400` for anything else, so a select built from a stale list can only
// ever offer too few options — never widen anything. Fetching `/openapi.json`
// into the browser just to populate a dropdown would add a request per page load
// and put the whole spec in the bundle for no authorization benefit.
//
// Keep in step with the `#[utoipa::path(operation_id = …)]` declarations in
// `backend/src/routes/`. `<unmatched>` is the audit catalog's own reserved id
// for a request that hit no declared route; it is offered because a global
// administrator investigating stray traffic needs exactly that bucket.

/** Every operation id an activity record can carry, grouped for the select. */
export const OPERATION_CATALOG: ReadonlyArray<{
  /** A stable group key; the catalog supplies its localized label. */
  group: 'canvas' | 'sessions' | 'environments' | 'auth' | 'operations' | 'system';
  ids: readonly string[];
}> = [
  {
    group: 'canvas',
    ids: [
      'canvas_overview',
      'canvas_repo_sessions',
      'canvas_create_session',
      'canvas_stop_session',
      'canvas_create_work_item',
      'canvas_session_outcomes',
      'canvas_outcome_blob',
      'create_repo',
      'uninstall_account',
    ],
  },
  {
    group: 'sessions',
    ids: [
      'observe_session',
      'list_session_runs',
      'download_session_logs',
      'session_log_manifest',
      'session_log_file',
      'chat_turn',
    ],
  },
  {
    group: 'environments',
    ids: [
      'list_user_environment_profiles',
      'get_user_environment_profile',
      'put_user_environment_profile',
      'delete_user_environment_profile',
    ],
  },
  {
    group: 'auth',
    ids: [
      'github_login',
      'github_login_callback',
      'github_refresh_token',
      'github_broader_connect',
      'github_broader_callback',
      'session_logs_oauth_callback',
    ],
  },
  {
    group: 'operations',
    ids: ['operations_list_activity', 'operations_list_sandboxes'],
  },
  {
    group: 'system',
    ids: ['github_app_webhook', '<unmatched>'],
  },
];

/** Flattened, for validating a URL-supplied `operation_id`. */
export const OPERATION_IDS: readonly string[] = OPERATION_CATALOG.flatMap((group) => group.ids);
