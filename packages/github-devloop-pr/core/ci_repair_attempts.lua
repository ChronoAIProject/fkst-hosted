local ci_failure_keys = require("devloop.ci_failure_keys")
local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local base_ids = require("devloop.base_ids")
local devloop_state = require("devloop.state")
local devloop_logging = require("devloop.logging")
local forge_validators = require("devloop.forge_validators")
local parsers_misc = require("devloop.parsers.misc")
local strings = require("contract.strings")
local C = {}

local function marker_attr(marker, name)
  return marker:match(name .. '="([^"]*)"')
end

local function valid_identity(value)
  return type(value) == "table"
    and strings.is_path_safe_key(value.proposal_id, devloop_base._max_key_len)
    and forge_validators.is_positive_pr_number(value.pr_number)
    and strings.is_bounded_string(value.version, devloop_base._max_dedup_len)
    and devloop_state.version_fix_round(value.version) > 0
end

local function valid_diagnostics(value)
  return forge_validators.is_git_sha(value.reviewed_head_sha)
    and ci_failure_keys.is_valid(value.ci_failure_key, devloop_base._max_dedup_len)
end

function C.marker(fix, outcome)
  if not valid_identity(fix) or not valid_diagnostics(fix) then
    error("github-devloop: ci-repair-attempt-invalid: invalid CI repair attempt marker")
  end
  local safe_outcome = strings.sanitize_key(outcome or "completed", false):gsub("/", "-")
  return '<!-- fkst:github-devloop:ci-repair-attempt:v1 proposal="' .. tostring(fix.proposal_id)
    .. '" pr="' .. tostring(fix.pr_number)
    .. '" version="' .. tostring(fix.version)
    .. '" fix_round="' .. tostring(devloop_state.version_fix_round(fix.version))
    .. '" reviewed_head_sha="' .. tostring(fix.reviewed_head_sha)
    .. '" ci_failure_key="' .. tostring(fix.ci_failure_key)
    .. '" outcome="' .. safe_outcome
    .. '" -->'
end

function C.comment_request(repo, fix, outcome, detail)
  local safe_detail = devloop_base.neutralize_untrusted_comment_text(detail or "")
  if safe_detail == "" then
    safe_detail = "CI repair round completed without publishing a repaired revision."
  end
  return entity_lib.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = fix.pr_number,
  }, "github-devloop recorded a completed own-CI repair round."
    .. "\n\nProposal: " .. tostring(fix.proposal_id)
    .. "\nPR: " .. tostring(fix.pr_number)
    .. "\nVersion: " .. tostring(fix.version)
    .. "\nFix round: " .. tostring(devloop_state.version_fix_round(fix.version))
    .. "\nReviewed head: " .. tostring(fix.reviewed_head_sha)
    .. "\nCI failure key: " .. tostring(fix.ci_failure_key)
    .. "\nOutcome: " .. tostring(outcome or "completed")
    .. "\n\nDetail:\n" .. safe_detail
    .. "\n\n" .. C.marker(fix, outcome)
    .. "\n" .. require("devloop.requests.shared").ai_sentinel, base_ids.dedup_key({
      "fix",
      "ci-repair-attempt",
      tostring(fix.proposal_id),
      tostring(fix.pr_number),
      tostring(fix.version),
    }), fix.source_ref)
end

function C.raise_attempt_record(repo, fix, outcome, detail)
  local comment_request = C.comment_request(repo, fix, outcome, detail)
  devloop_logging.log_raise("fix", fix.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
end

function C.fact(comments, identity)
  if type(comments) ~= "table" or not valid_identity(identity) then
    return nil
  end
  for _, comment in ipairs(parsers_misc._trusted_marker_comments(comments)) do
    for marker in tostring(comment.body or ""):gmatch("<!%-%- fkst:github%-devloop:ci%-repair%-attempt:v1.-%-%->") do
      local fact = {
        proposal_id = marker_attr(marker, "proposal"),
        pr_number = marker_attr(marker, "pr"),
        version = marker_attr(marker, "version"),
        fix_round = tonumber(marker_attr(marker, "fix_round")),
        reviewed_head_sha = marker_attr(marker, "reviewed_head_sha"),
        ci_failure_key = marker_attr(marker, "ci_failure_key"),
        outcome = marker_attr(marker, "outcome"),
        comment_created_at = parsers_misc._comment_created_at(comment),
      }
      if fact.proposal_id == tostring(identity.proposal_id)
        and fact.pr_number == tostring(identity.pr_number)
        and fact.version == tostring(identity.version)
        and fact.fix_round == devloop_state.version_fix_round(fact.version)
        and fact.fix_round > 0
        and valid_diagnostics(fact)
        and strings.is_bounded_string(fact.outcome, devloop_base._max_key_len) then
        return fact
      end
    end
  end
  return nil
end

return C
