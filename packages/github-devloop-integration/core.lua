local saga_conformance = require("devloop.saga_conformance")
local parsers_misc = require("devloop.parsers.misc")
local parsers_pr = require("devloop.parsers.pr")
local M
local wiring = require("core.devloop_wiring")

-- fkst.toml conformance hook: function = "core.saga_conformance_errors" (delegates to typed devloop.saga_conformance.errors)
local function saga_conformance_errors()
  return saga_conformance.errors(M)
end

M = {
  saga_conformance_errors = saga_conformance_errors,
}

function M.parse_pr_view_merge(stdout)
  return parsers_pr.parse_pr_view_merge(M, stdout)
end

function M.rollup_failure_gate_sha(pr)
  return parsers_misc.rollup_failure_gate_sha(M, pr)
end

local base = require("devloop.base")
local function dept_exec_sync(...) return exec_sync(...) end
M.judgment_worktree = function(...) return base.judgment_worktree_with_exec(M, dept_exec_sync, ...) end
M.read_env_command = function(...) return base.read_env_command(M, ...) end
M.read_env = function(...) return base.read_env(M, ...) end
M.parse_name_only_paths = function(...) return base.parse_name_only_paths(M, ...) end
M.configure_trusted_bot_login = function(...) return base.configure_trusted_bot_login(M, ...) end
M.assert_trusted_bot_configured = function(...) return base.assert_trusted_bot_configured(M, ...) end
M.safe_updated_at = function(...) return base.safe_updated_at(M, ...) end
M.safe_pr_review_repo_segment = function(...) return base.safe_pr_review_repo_segment(M, ...) end
M.is_intake_held = function(...) return base.is_intake_held(M, ...) end
M.safe_head_segment = function(...) return base.safe_head_segment(M, ...) end
M.pr_review_proposal_id = function(...) return base.pr_review_proposal_id(M, ...) end
M.parse_pr_review_proposal_id = function(...) return base.parse_pr_review_proposal_id(M, ...) end
M.parse_issue_source_ref = function(...) return base.parse_issue_source_ref(M, ...) end
M.is_safe_proposal_ref = function(...) return base.is_safe_proposal_ref(M, ...) end
M.is_safe_consensus_result_ref = function(...) return base.is_safe_consensus_result_ref(M, ...) end
M.is_safe_pr_review_result_ref = function(...) return base.is_safe_pr_review_result_ref(M, ...) end
M.proposal_dedup_key = function(...) return base.proposal_dedup_key(M, ...) end
M.intake_dedup_key = function(...) return base.intake_dedup_key(M, ...) end
M.intake_candidate_delivery_dedup_key = function(...) return base.intake_candidate_delivery_dedup_key(M, ...) end
M.implement_version_mismatch_key = function(...) return base.implement_version_mismatch_key(M, ...) end
M.intake_decision_dedup_key = function(...) return base.intake_decision_dedup_key(M, ...) end
M.ci_selfheal_once_key = function(...) return base.ci_selfheal_once_key(M, ...) end
M.ci_missing_status_first_observed_key = function(...) return base.ci_missing_status_first_observed_key(M, ...) end
M.observe_lock_key = function(...) return base.observe_lock_key(M, ...) end
M.transition_lock_key = function(...) return base.transition_lock_key(M, ...) end
M.result_lock_key = function(...) return base.result_lock_key(M, ...) end
M.review_result_lock_key = function(...) return base.review_result_lock_key(M, ...) end
M.review_lock_key = function(...) return base.review_lock_key(M, ...) end
M.loop_lock_key = function(...) return base.loop_lock_key(M, ...) end
M.implement_lock_key = function(...) return base.implement_lock_key(M, ...) end
M.safe_issue_slug = function(...) return base.safe_issue_slug(M, ...) end
M.implement_branch = function(...) return base.implement_branch(M, ...) end
M.implement_worktree_path = function(...) return base.implement_worktree_path(M, ...) end
M.path_under_runtime_root = function(...) return base.path_under_runtime_root(M, ...) end
M.judgment_worktree_path = function(...) return base.judgment_worktree_path(M, ...) end
M.max_body_len = function(...) return base.max_body_len(M, ...) end
M.render_template = function(...) return base.render_template(M, ...) end
M.quote_untrusted_prompt_text = function(...) return base.quote_untrusted_prompt_text(M, ...) end
M.gh_exec_opts = function(...) return base.gh_exec_opts(M, ...) end
M.trusted_bot_login = function(...) return base.trusted_bot_login(M, ...) end
M.judgment_codex_opts = base.judgment_codex_opts
M._max_key_len = base._max_key_len
M._max_dedup_len = base._max_dedup_len
M._max_title_len = base._max_title_len
M._max_body_len = base._max_body_len
M._max_comments_len = base._max_comments_len
M._max_meta_reason_len = base._max_meta_reason_len
M._max_framing_len = base._max_framing_len
M._max_impl_output_len = base._max_impl_output_len
M._max_blocking_gap_len = base._max_blocking_gap_len
M._max_review_ledger_len = base._max_review_ledger_len
M._max_pr_issue_context_len = base._max_pr_issue_context_len
M._max_pr_title_len = base._max_pr_title_len
M._action_label = base._action_label
M._intake_label = base._intake_label
M._class_label = base._class_label
M._reason_label = base._reason_label
M._verdict_label = base._verdict_label
M._reply_label = base._reply_label
M._untrusted_issue_data_begin = base._untrusted_issue_data_begin
M._untrusted_issue_data_end = base._untrusted_issue_data_end
M._test_bot_login = base._test_bot_login
M._enabled_label = base._enabled_label
M._tracking_label = base._tracking_label
M._hold_label = base._hold_label
M._thinking_label = base._thinking_label
M._ready_label = base._ready_label
M._implementing_label = base._implementing_label
M._awaiting_pr_label = base._awaiting_pr_label
M._pr_open_label = base._pr_open_label
M._reviewing_label = base._reviewing_label
M._merge_ready_label = base._merge_ready_label
M._merging_label = base._merging_label
M._merged_label = base._merged_label
M._fixing_label = base._fixing_label
M._review_meta_label = base._review_meta_label
M._impl_failed_label = base._impl_failed_label
M._blocked_label = base._blocked_label
M._blocked_on_dependency_label = base._blocked_on_dependency_label
M._label_colors = base._label_colors
M._shell_single_quote = base._shell_single_quote
M._neutralize_fkst_markers = base._neutralize_fkst_markers
M._has_value = base._has_value
M._is_review_meta_action = base._is_review_meta_action
M.fix_reflection_checkpoint_round = base.fix_reflection_checkpoint_round
require("forge.github_debug_stamp").install(M)
require("devloop.commands").install(M)
require("forge.merge_commands").install(M)
local git_mechanics = require("devloop.git_mechanics")
local function dept_exec_argv(...) return exec_argv(...) end
M._git = require("forge.git").new(dept_exec_argv)
M.repo_ref_store_lock_key = function(...) return git_mechanics.repo_ref_store_lock_key(M, ...) end
M.with_repo_ref_store_lock = function(...) return git_mechanics.with_repo_ref_store_lock(M, ...) end
M.run_required = function(...) return git_mechanics.run_required(M, ...) end
M.fetch_branch = function(...) return git_mechanics.fetch_branch(M, ...) end
M.fetch_branches = function(...) return git_mechanics.fetch_branches(M, ...) end
M.remote_head = function(...) return git_mechanics.remote_head(M, ...) end
M.is_ancestor = function(...) return git_mechanics.is_ancestor(M, ...) end
M.git_is_ancestor = function(...) return git_mechanics.git_is_ancestor(M, ...) end
M.git_merge_no_ff = function(...) return git_mechanics.git_merge_no_ff(M, ...) end
M.git_fast_forward = function(...) return git_mechanics.git_fast_forward(M, ...) end
M.git_remote_trees_equal_quiet = function(...) return git_mechanics.git_remote_trees_equal_quiet(M, ...) end
M.git_trees_equal_quiet = function(...) return git_mechanics.git_trees_equal_quiet(M, ...) end
M.current_base_head = function(...) return git_mechanics.current_base_head(M, ...) end
M.has_empty_resolution_delta = function(...) return git_mechanics.has_empty_resolution_delta(M, ...) end
M.current_branch_head_sha = function(...) return git_mechanics.current_branch_head_sha(M, ...) end
M.git_push_branch_force_with_lease = function(...) return git_mechanics.git_push_branch_force_with_lease(M, ...) end
M.git_push_branch_update = function(...) return git_mechanics.git_push_branch_update(M, ...) end
M.git_push_worktree_branch_update = function(...) return git_mechanics.git_push_worktree_branch_update(M, ...) end
M.git_unmerged_paths = function(...) return git_mechanics.git_unmerged_paths(M, ...) end
M.git_diff_check = function(...) return git_mechanics.git_diff_check(M, ...) end
M.git_diff_cached_check = function(...) return git_mechanics.git_diff_cached_check(M, ...) end
M.git_conflict_markers = function(...) return git_mechanics.git_conflict_markers(M, ...) end
M.git_commit_message_file = function(...) return git_mechanics.git_commit_message_file(M, ...) end
M.git_worktree_remove = function(...) return git_mechanics.git_worktree_remove(M, ...) end
M.runtime_root = function(...) return git_mechanics.runtime_root_with_exec(M, dept_exec_sync, ...) end
require("forge.merge").install(M)
require("devloop.logging").install(M)
require("devloop.state").install(M)
local entity = require("devloop.entity")
M.pr_source_ref = function(...) return entity.pr_source_ref(M, ...) end
M.issue_source_ref = function(...) return entity.issue_source_ref(M, ...) end
M.build_entity_comment_request = function(...) return entity.build_entity_comment_request(M, ...) end
M.linked_pr_surface_snapshot = function(...) return entity.linked_pr_surface_snapshot(M, ...) end
M.pr_proposal_id = function(...) return entity.pr_proposal_id(M, ...) end
M.parse_pr_proposal_id = function(...) return entity.parse_pr_proposal_id(M, ...) end
M.pr_transition_lock_key = function(...) return entity.pr_transition_lock_key(M, ...) end
M.merge_lane_lock_key = function(...) return entity.merge_lane_lock_key(M, ...) end
M.parse_entity_proposal_id = function(...) return entity.parse_entity_proposal_id(M, ...) end
M.is_safe_entity_proposal_ref = function(...) return entity.is_safe_entity_proposal_ref(M, ...) end
M.transition_lock_key = function(...) return entity.transition_lock_key(M, ...) end
M.observe_lock_key = function(...) return entity.observe_lock_key(M, ...) end
M.result_lock_key = function(...) return entity.result_lock_key(M, ...) end
M.review_result_lock_key = function(...) return entity.review_result_lock_key(M, ...) end
M.review_lock_key = function(...) return entity.review_lock_key(M, ...) end
M.loop_lock_key = function(...) return entity.loop_lock_key(M, ...) end
M.implement_lock_key = function(...) return entity.implement_lock_key(M, ...) end
M.pr_native_origin = function(...) return entity.pr_native_origin(M, ...) end
local prompts = require("devloop.prompts")
prompts.install(M, wiring.prompts(), { sync_conflict = true })
local github_proxy_entity_view = require("devloop.github_proxy_entity_view")
M.cached_entity_view = function(...) return github_proxy_entity_view.cached_entity_view(M, ...) end
M.fetch_pr_view_origin = function(...) return github_proxy_entity_view.fetch_pr_view_origin(M, ...) end
M.invalidate_entity_after_write = function(...) return github_proxy_entity_view.invalidate_entity_after_write(M, ...) end
require("core.branches").install(M)
require("core.sync_conflict").install(M)
require("core.rollup_health").install(M)
require("core.release_notes").install(M)

return M
