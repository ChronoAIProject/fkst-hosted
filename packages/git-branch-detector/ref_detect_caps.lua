local core = require("core")

return {
  git_ref_changed_payload = core.git_ref_changed_payload,
  lookup_error_class = core.lookup_error_class,
  lookup_failure_fact = core.lookup_failure_fact,
  lookup_remote_branch_sha = core.lookup_remote_branch_sha,
  observed_at = core.observed_at,
  parse_watch_refs = core.parse_watch_refs,
}
