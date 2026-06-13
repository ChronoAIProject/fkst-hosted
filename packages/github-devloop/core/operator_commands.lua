local S = {}

function S.install(M)
local ai_sentinel = "⟦AI:FKST⟧"

local function command_key(comment, fallback_index)
  if type(comment) == "table" and comment.id ~= nil and tostring(comment.id) ~= "" then
    return M._dedup_key({
      "operator-command",
      tostring(comment.id),
    })
  end
  local created = M._comment_created_at(comment) or "unknown-time"
  local author = M._comment_author_login(comment) or "unknown-author"
  return M._dedup_key({
    "operator-command",
    tostring(author),
    tostring(created),
    tostring(fallback_index or 0),
    M._comment_body(comment),
  })
end

local function first_command_line(body)
  for line in tostring(body or ""):gmatch("[^\r\n]+") do
    local trimmed = M._trim(line):lower()
    if trimmed ~= "" then
      return trimmed
    end
  end
  return ""
end

local function parse_command(body)
  local line = first_command_line(body)
  local command = line:match("^fkst:%s*([%w_-]+)")
  if command == "rereview" or command == "reready" or command == "reintake" or command == "reimplement" then
    return {
      command = command,
    }
  end
  if command == "dependency-waiver" then
    local number = tonumber(line:match("^fkst:%s*dependency%-waiver%s+(%d+)%s*$") or "")
    if M._is_positive_pr_number(number) then
      return {
        command = command,
        blocker_number = math.floor(number),
      }
    end
  end
  return nil
end

function M.operator_command_fact(comments, command_name)
  if type(comments) ~= "table" then
    return nil
  end
  local latest = nil
  for index, comment in ipairs(comments) do
    local parsed = parse_command(M._comment_body(comment))
    if parsed ~= nil and parsed.command == command_name then
      if M._is_trusted_comment(comment) then
        latest = {
          command = parsed.command,
          key = command_key(comment, index),
          author_login = M._comment_author_login(comment),
          created_at = M._comment_created_at(comment),
          body = M._comment_body(comment),
          blocker_number = parsed.blocker_number,
        }
      else
        M.log_line("info", "operator_command", "IGNORED", {
          "command=" .. tostring(parsed.command),
          "reason=untrusted-author",
          "ignored_author=" .. tostring(M._comment_author_login(comment) or ""),
          "trusted_bot=" .. tostring(M.trusted_bot_login()),
        })
      end
    end
  end
  return latest
end

function M.operator_rereview_version(current_version, head_sha)
  if not M._is_git_sha(head_sha) then
    error("github-devloop: invalid operator rereview head sha")
  end
  local base = tostring(current_version or "")
  local next_n = M.version_review_loop_round(base) + 1
  return base .. "/review-loop/" .. tostring(next_n) .. "/rereview/" .. tostring(next_n) .. "/" .. tostring(head_sha)
end

function M.has_operator_command_response(comments, command)
  if type(comments) ~= "table" or type(command) ~= "table" then
    return false
  end
  local marker = '<!-- fkst:github-devloop:operator-command:v1 command="'
    .. tostring(command.command)
    .. '" key="' .. tostring(command.key)
    .. '"'
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    if M._comment_body(comment):find(marker, 1, true) ~= nil then
      return true
    end
  end
  return false
end

function M.operator_command_marker(command, outcome, reason)
  if type(command) ~= "table"
    or (command.command ~= "rereview"
      and command.command ~= "reready"
      and command.command ~= "reintake"
      and command.command ~= "reimplement"
      and command.command ~= "dependency-waiver") then
    error("github-devloop: invalid operator command marker")
  end
  if outcome ~= "applied" and outcome ~= "refused" then
    error("github-devloop: invalid operator command outcome")
  end
  local safe_reason = M.sanitize_key(reason or outcome, false):gsub("/", "-")
  return '<!-- fkst:github-devloop:operator-command:v1 command="' .. tostring(command.command)
    .. '" key="' .. tostring(command.key)
    .. '" outcome="' .. tostring(outcome)
    .. '" reason="' .. tostring(safe_reason)
    .. '" -->'
end

function M.build_operator_rereview_comment_request(repo, pr_number, proposal_id, new_version, command, source_ref)
  local state_marker = M.state_marker(proposal_id, "reviewing", new_version)
  local marker = M.operator_command_marker(command, "applied", "rereview")
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = pr_number,
  }, "github-devloop operator command accepted: rereview"
    .. "\n\n" .. state_marker
    .. "\n" .. marker
    .. "\n" .. ai_sentinel, M._dedup_key({
    "operator-command",
    "comment",
    tostring(command.key),
    "applied",
    tostring(new_version),
  }), source_ref)
end

function M.build_operator_issue_rereview_comment_request(repo, issue_number, command, proposal, source_ref)
  local marker = M.operator_command_marker(command, "applied", "rereview")
  return M.build_entity_comment_request({
    kind = "issue",
    repo = repo,
    number = issue_number,
  }, "github-devloop operator command accepted: rereview"
    .. "\n\n" .. marker
    .. "\n" .. ai_sentinel, M._dedup_key({
    "operator-command",
    "comment",
    tostring(command.key),
    "applied",
    tostring(proposal and proposal.dedup_key or ""),
  }), source_ref)
end

function M.build_operator_issue_reready_comment_request(repo, issue_number, command, outcome_reason, source_ref)
  local marker = M.operator_command_marker(command, "applied", outcome_reason or "reready")
  return M.build_entity_comment_request({
    kind = "issue",
    repo = repo,
    number = issue_number,
  }, "github-devloop operator command accepted: reready"
    .. "\n\n" .. marker
    .. "\n" .. ai_sentinel, M._dedup_key({
    "operator-command",
    "comment",
    tostring(command.key),
    "applied",
    tostring(outcome_reason or "reready"),
  }), source_ref)
end

function M.build_operator_issue_reimplement_comment_request(repo, issue_number, command, attempt, source_ref)
  local marker = M.operator_command_marker(command, "applied", "reimplement")
  return M.build_entity_comment_request({
    kind = "issue",
    repo = repo,
    number = issue_number,
  }, "github-devloop operator command accepted: reimplement"
    .. "\n\nRetry attempt: " .. tostring(attempt)
    .. "\n\n" .. marker
    .. "\n" .. ai_sentinel, M._dedup_key({
    "operator-command",
    "comment",
    tostring(command.key),
    "applied",
    "reimplement",
    tostring(attempt),
  }), source_ref)
end

function M.build_operator_issue_dependency_waiver_comment_request(repo, issue_number, command, proposal_id, version, blocker_number, source_ref)
  local waiver_marker = M.dependency_waiver_marker(proposal_id, version, blocker_number, "operator-waiver")
  local command_marker = M.operator_command_marker(command, "applied", "dependency-waiver")
  return M.build_entity_comment_request({
    kind = "issue",
    repo = repo,
    number = issue_number,
  }, "github-devloop operator command accepted: dependency-waiver"
    .. "\n\n" .. waiver_marker
    .. "\n" .. command_marker
    .. "\n" .. ai_sentinel, M._dedup_key({
    "operator-command",
    "comment",
    tostring(command.key),
    "applied",
    "dependency-waiver",
    tostring(version),
    tostring(blocker_number),
  }), source_ref)
end

function M.build_operator_issue_reintake_comment_request(repo, issue_number, command, candidate, source_ref)
  local marker = M.operator_command_marker(command, "applied", "reintake")
  return M.build_entity_comment_request({
    kind = "issue",
    repo = repo,
    number = issue_number,
  }, "github-devloop operator command accepted: reintake"
    .. "\n\n" .. marker
    .. "\n" .. ai_sentinel, M._dedup_key({
    "operator-command",
    "comment",
    tostring(command.key),
    "applied",
    tostring(candidate and candidate.dedup_key or "reintake"),
  }), source_ref)
end

function M.build_operator_command_refusal_request(repo, pr_number, command, reason, source_ref)
  local safe_reason = M.neutralize_untrusted_comment_text(reason or "invalid command state")
  local marker = M.operator_command_marker(command, "refused", reason)
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = pr_number,
  }, "github-devloop operator command refused: " .. safe_reason
    .. "\n\n" .. marker
    .. "\n" .. ai_sentinel, M._dedup_key({
    "operator-command",
    "comment",
    tostring(command.key),
    "refused",
    tostring(reason or "invalid"),
  }), source_ref)
end

function M.build_operator_issue_command_refusal_request(repo, issue_number, command, reason, source_ref)
  local safe_reason = M.neutralize_untrusted_comment_text(reason or "invalid command state")
  local marker = M.operator_command_marker(command, "refused", reason)
  return M.build_entity_comment_request({
    kind = "issue",
    repo = repo,
    number = issue_number,
  }, "github-devloop operator command refused: " .. safe_reason
    .. "\n\n" .. marker
    .. "\n" .. ai_sentinel, M._dedup_key({
    "operator-command",
    "comment",
    tostring(command.key),
    "refused",
    tostring(reason or "invalid"),
  }), source_ref)
end
end

return S
