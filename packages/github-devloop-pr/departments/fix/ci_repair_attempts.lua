local ci_failure_keys = require("devloop.ci_failure_keys")
local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local base_ids = require("devloop.base_ids")
local requests_review = require("devloop.requests.review")
local requests_labels = require("devloop.requests.labels")
local devloop_state = require("devloop.state")
local devloop_logging = require("devloop.logging")
local forge_validators = require("devloop.forge_validators")
local strings = require("contract.strings")

local C = {}

local function marker_attr(marker, name)
  return marker:match(name .. '="([^"]*)"')
end

function C.marker(fix, outcome)
  if type(fix) ~= "table"
    or not ci_failure_keys.is_valid(fix.ci_failure_key, devloop_base._max_dedup_len)
    or not strings.is_path_safe_key(fix.work_unit_key, devloop_base._max_dedup_len)
    or not forge_validators.is_positive_pr_number(fix.pr_number)
    or not forge_validators.is_git_sha(fix.reviewed_head_sha) then
    error("github-devloop: ci-repair-attempt-invalid: invalid CI repair attempt marker")
  end
  local safe_outcome = strings.sanitize_key(outcome or "completed", false):gsub("/", "-")
  return '<!-- fkst:github-devloop:ci-repair-attempt:v1 proposal="' .. tostring(fix.proposal_id)
    .. '" pr="' .. tostring(fix.pr_number)
    .. '" reviewed_head_sha="' .. tostring(fix.reviewed_head_sha)
    .. '" ci_failure_key="' .. tostring(fix.ci_failure_key)
    .. '" work_unit_key="' .. tostring(fix.work_unit_key)
    .. '" outcome="' .. safe_outcome
    .. '" -->'
end

function C.comment_request(repo, fix, outcome, detail)
  local safe_detail = devloop_base.neutralize_untrusted_comment_text(detail or "")
  if safe_detail == "" then
    safe_detail = "CI repair attempt completed without publishing a repaired revision."
  end
  return entity_lib.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = fix.pr_number,
  }, "github-devloop recorded the bounded own-CI repair attempt."
    .. "\n\nProposal: " .. tostring(fix.proposal_id)
    .. "\nPR: " .. tostring(fix.pr_number)
    .. "\nReviewed head: " .. tostring(fix.reviewed_head_sha)
    .. "\nCI failure key: " .. tostring(fix.ci_failure_key)
    .. "\nWork unit: " .. tostring(fix.work_unit_key)
    .. "\nOutcome: " .. tostring(outcome or "completed")
    .. "\n\nDetail:\n" .. safe_detail
    .. "\n\n" .. C.marker(fix, outcome)
    .. "\n" .. require("devloop.requests.shared").ai_sentinel, base_ids.dedup_key({
      "fix",
      "ci-repair-attempt",
      tostring(fix.proposal_id),
      tostring(fix.pr_number),
      tostring(fix.reviewed_head_sha),
      tostring(fix.ci_failure_key),
      tostring(fix.work_unit_key),
      tostring(outcome or "completed"),
    }), fix.source_ref)
end

function C.raise_attempt_record(repo, fix, outcome, detail)
  local comment_request = C.comment_request(repo, fix, outcome, detail)
  devloop_logging.log_raise("fix", fix.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
end

function C.raise_blocked(repo, issue_number, fix, reason, detail)
  local comment_request = requests_review.build_ci_failure_unrepaired_blocked_request(repo, fix, reason, detail)
  local label_request = issue_number ~= nil and requests_labels.build_state_label_request(repo,
    issue_number,
    "blocked",
    fix.dedup_key .. "/label/own-ci-red-unrepaired",
    entity_lib.issue_source_ref(repo, issue_number)
  ) or nil
  local add_labels, remove_labels = devloop_state.state_label_changes("blocked")
  local raised = {
    "github-proxy.github_pr_comment_request",
  }
  if label_request ~= nil then
    table.insert(raised, "github-proxy.github_issue_label_request")
  end
  devloop_logging.log_apply("fix", fix.proposal_id, "blocked", comment_request.handoff.version, { add = add_labels, remove = remove_labels }, raised)
  devloop_logging.log_raise("fix", fix.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  if label_request ~= nil then
    devloop_logging.log_raise("fix", fix.proposal_id, "github-proxy.github_issue_label_request", label_request)
  end
end

function C.fact(comments, fix)
  if type(comments) ~= "table" or type(fix) ~= "table" then
    return nil
  end
  for _, comment in ipairs(require("devloop.parsers.misc")._trusted_marker_comments(comments)) do
    for marker in tostring(comment.body or ""):gmatch("<!%-%- fkst:github%-devloop:ci%-repair%-attempt:v1.-%-%->") do
      local fact = {
        proposal_id = marker_attr(marker, "proposal"),
        pr_number = marker_attr(marker, "pr"),
        reviewed_head_sha = marker_attr(marker, "reviewed_head_sha"),
        ci_failure_key = marker_attr(marker, "ci_failure_key"),
        work_unit_key = marker_attr(marker, "work_unit_key"),
        outcome = marker_attr(marker, "outcome"),
      }
      if fact.proposal_id == tostring(fix.proposal_id)
        and fact.pr_number == tostring(fix.pr_number)
        and fact.reviewed_head_sha == tostring(fix.reviewed_head_sha)
        and fact.ci_failure_key == tostring(fix.ci_failure_key)
        and fact.work_unit_key == tostring(fix.work_unit_key)
        and ci_failure_keys.is_valid(fact.ci_failure_key, devloop_base._max_dedup_len)
        and strings.is_path_safe_key(fact.work_unit_key, devloop_base._max_dedup_len)
        and forge_validators.is_git_sha(fact.reviewed_head_sha)
        and strings.is_bounded_string(fact.outcome, devloop_base._max_key_len) then
        return fact
      end
    end
  end
  return nil
end

return C
