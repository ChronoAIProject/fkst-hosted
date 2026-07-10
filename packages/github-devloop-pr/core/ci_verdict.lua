local forge_validators = require("devloop.forge_validators")
local parsers_pr = require("devloop.parsers.pr")

local C = {}
local production_handle = require("devloop.github_factory").production_handle
local parse_pr_view_merge = parsers_pr.parse_pr_view_merge

C.OWN_CI_RED = "OWN_CI_RED"

local function classify_pr_ci_gate(pr, opts)
  return require("core").classify_pr_ci_gate(pr, opts)
end

function C.with_current_classification(repo, pr_number, expected_head, effect, ctx)
  if tostring(repo or "") == "" or not forge_validators.is_positive_pr_number(pr_number) then
    error("github-devloop: ci-classification-identity-invalid: repository and pull request are required")
  end
  if not forge_validators.is_git_sha(expected_head) then
    error("github-devloop: ci-classification-head-invalid: an expected PR head is required")
  end
  if type(effect) ~= "function" then
    error("github-devloop: ci-classification-effect-invalid: a synchronous effect is required")
  end
  local current_view, command_result = production_handle().gh_pr_view_merge(repo, pr_number, 30)
  local error_class = type(ctx) == "table" and ctx.error_class or "CI-bearing PR view failed"
  if current_view == nil then
    error("github-devloop: ci-classification-read-failed: " .. tostring(error_class)
      .. ": " .. tostring(command_result and command_result.stderr or "missing result"))
  end
  local current_pr = parse_pr_view_merge(current_view)
  current_pr.number = tonumber(pr_number)
  if current_pr.status_check_rollup_present ~= true or type(current_pr.status_check_rollup) ~= "table" then
    error("github-devloop: ci-classification-facts-missing: statusCheckRollup is required")
  end
  local head_sha = tostring(current_pr.head_sha or "")
  if not forge_validators.is_git_sha(head_sha) then
    error("github-devloop: ci-classification-observed-head-invalid: a current PR head is required")
  end
  if head_sha ~= tostring(expected_head) then
    return nil, "head-mismatch", current_pr
  end

  local classified = classify_pr_ci_gate(current_pr, {
    repo = repo,
    dept = type(ctx) == "table" and ctx.dept or nil,
    proposal_id = type(ctx) == "table" and ctx.proposal_id or nil,
  })
  if type(classified) ~= "table" or tostring(classified.kind or "") == "" then
    error("github-devloop: ci-classification-invalid: canonical classification failed")
  end
  if classified.kind == C.OWN_CI_RED and tostring(classified.ci_failure_key or "") == "" then
    error("github-devloop: ci-classification-own-ci-key-missing: own-CI red requires a failure key")
  end

  return effect({
    repo = tostring(repo),
    pr_number = tonumber(pr_number),
    head_sha = head_sha,
    kind = classified.kind,
    reason = classified.reason,
    ci_failure_key = classified.ci_failure_key,
    current_pr = current_pr,
  })
end

return C
