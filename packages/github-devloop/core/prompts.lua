local S = {}

function S.install(M)
function M.build_implement_prompt(proposal_id, current)
  local prompt = require("prompts.implement")
  return M.render_template(prompt.template, {
    proposal_id = M.neutralize_untrusted_prompt_text(proposal_id),
    title = M.neutralize_untrusted_prompt_text(current.title),
    body = M.neutralize_untrusted_prompt_text(current.body),
  })
end

function M.build_fix_prompt(fix, current_issue, review_reason)
  local prompt = require("prompts.fix")
  return M.render_template(prompt.template, {
    proposal_id = M.neutralize_untrusted_prompt_text(fix.proposal_id),
    review_proposal_id = M.neutralize_untrusted_prompt_text(fix.review_proposal_id),
    reviewed_head_sha = M.neutralize_untrusted_prompt_text(fix.reviewed_head_sha),
    title = M.neutralize_untrusted_prompt_text(current_issue.title),
    body = M.neutralize_untrusted_prompt_text(current_issue.body),
    review_feedback = M.neutralize_untrusted_prompt_text(review_reason),
  })
end

function M.build_sync_conflict_prompt(conflict)
  local prompt = require("prompts.sync_conflict")
  return M.render_template(prompt.template, {
    repo = M.neutralize_untrusted_prompt_text(conflict.repo),
    upstream_branch = M.neutralize_untrusted_prompt_text(conflict.upstream_branch),
    integration_branch = M.neutralize_untrusted_prompt_text(conflict.integration_branch),
    upstream_sha = M.neutralize_untrusted_prompt_text(conflict.upstream_sha),
    integration_sha = M.neutralize_untrusted_prompt_text(conflict.integration_sha),
  })
end

function M.build_review_meta_prompt(review_meta, current_issue)
  local prompt = require("prompts.review_meta")
  local comments = table.concat(M.comment_bodies(current_issue.comments), "\n\n--- comment ---\n\n")
  if #comments > M._max_comments_len then
    comments = comments:sub(1, M._max_comments_len)
  end

  return M.render_template(prompt.template, {
    proposal_id = M.neutralize_untrusted_prompt_text(review_meta.proposal_id),
    review_proposal_id = M.neutralize_untrusted_prompt_text(review_meta.review_proposal_id),
    title = M.neutralize_untrusted_prompt_text(current_issue.title),
    body = M.neutralize_untrusted_prompt_text(current_issue.body),
    comments = M.neutralize_untrusted_prompt_text(comments),
  })
end

function M.build_intake_prompt(proposal_id, current)
  local prompt = require("prompts.intake")
  local comments = table.concat(M.comment_bodies(current.comments), "\n\n--- comment ---\n\n")
  if #comments > M._max_comments_len then
    comments = comments:sub(1, M._max_comments_len)
  end

  return M.render_template(prompt.template, {
    proposal_id = M.neutralize_untrusted_prompt_text(proposal_id),
    title = M.quote_untrusted_prompt_text(current.title),
    body = M.quote_untrusted_prompt_text(current.body),
    comments = M.quote_untrusted_prompt_text(comments),
  })
end

local function is_intake_action(value)
  return value == "enable" or value == "decline"
end

function M.parse_intake_action(stdout)
  local text = tostring(stdout or "")
  local lines = {}
  for line in (text .. "\n"):gmatch("(.-)\n") do
    table.insert(lines, line)
  end
  while #lines > 0 and M._trim(lines[#lines]) == "" do
    table.remove(lines)
  end
  if #lines ~= 2 then
    return nil
  end

  local action = lines[1]:match("^" .. M._intake_label .. " (enable)$")
    or lines[1]:match("^" .. M._intake_label .. " (decline)$")
  local reason = lines[2]:match("^" .. M._reason_label .. " (.+)$")
  if action == nil or not is_intake_action(action) then
    return nil
  end
  if reason == nil or M._trim(reason) == "" then
    return nil
  end
  if not M._is_bounded_string(reason, M._max_meta_reason_len) then
    return nil
  end
  return {
    action = action,
    reason = M._trim(reason),
  }
end

function M.parse_review_meta_action(stdout)
  local text = tostring(stdout or "")

  local action = nil
  local action_count = 0
  local action_index = nil
  local reason = nil
  local reason_count = 0
  local reason_index = nil
  local index = 0
  for line in (text .. "\n"):gmatch("(.-)\n") do
    index = index + 1

    if line:match("^%s*" .. M._action_label) ~= nil then
      local token = line:match("^%s*" .. M._action_label .. "%s*(%a+)%s*$")
      if token == nil or not M._is_review_meta_action(token:lower()) then
        return nil
      end
      action = token:lower()
      action_count = action_count + 1
      action_index = index
    end

    if line:match("^%s*" .. M._reason_label) ~= nil then
      local captured = line:match("^%s*" .. M._reason_label .. "%s*(.+)$")
      if captured == nil or M._trim(captured) == "" then
        return nil
      end
      reason = M._trim(captured)
      reason_count = reason_count + 1
      reason_index = index
    end
  end

  if action_count ~= 1 or reason_count ~= 1 then
    return nil
  end
  if action == nil or reason == nil then
    return nil
  end
  if reason_index ~= action_index + 1 then
    return nil
  end
  if not M._is_bounded_string(reason, M._max_meta_reason_len) then
    return nil
  end

  return {
    action = action,
    reason = reason,
  }
end
end

return S
