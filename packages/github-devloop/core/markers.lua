local S = {}

function S.install(M)
function M.review_meta_marker(issue_proposal_id, dedup_key, action, version)
  local fields = ""
  if action ~= nil then
    if not M._is_review_meta_action(action) then
      error("github-devloop: invalid review-meta action")
    end
    fields = fields .. '" action="' .. tostring(action)
  end
  if version ~= nil then
    fields = fields .. '" version="' .. tostring(version)
  end
  return '<!-- fkst:github-devloop:review-meta:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" dedup="' .. tostring(dedup_key)
    .. fields
    .. '" -->'
end

function M.fix_marker(issue_proposal_id, review_proposal_id, review_dedup_key, old_head_sha, new_head_sha)
  if not M._is_git_sha(old_head_sha) or not M._is_git_sha(new_head_sha) then
    error("github-devloop: invalid fix head sha")
  end
  return '<!-- fkst:github-devloop:fix:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" review_proposal="' .. tostring(review_proposal_id)
    .. '" review_dedup="' .. tostring(review_dedup_key)
    .. '" old_head_sha="' .. tostring(old_head_sha)
    .. '" new_head_sha="' .. tostring(new_head_sha)
    .. '" -->'
end

function M.merge_gate_marker(issue_proposal_id, pr_number, version, review_proposal_id, review_dedup_key, head_sha, reason)
  if not M._is_positive_pr_number(pr_number) or not M._is_git_sha(head_sha) then
    error("github-devloop: invalid merge-gate marker")
  end
  return '<!-- fkst:github-devloop:merge-gate:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" pr="' .. tostring(pr_number)
    .. '" version="' .. tostring(version)
    .. '" review_proposal="' .. tostring(review_proposal_id)
    .. '" review_dedup="' .. tostring(review_dedup_key)
    .. '" head_sha="' .. tostring(head_sha)
    .. '" reason="' .. tostring(M.sanitize_key(reason or "gate-failed", false):gsub("/", "-"))
    .. '" -->'
end

function M.implementing_marker(proposal_id, dedup_key, branch, head_sha, base_branch, base_sha)
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  if not M._is_git_sha(head_sha) then
    error("github-devloop: invalid head sha")
  end
  if not M._is_git_ref_safe(base_branch) then
    error("github-devloop: invalid base branch")
  end
  if not M._is_git_sha(base_sha) then
    error("github-devloop: invalid base sha")
  end
  return '<!-- fkst:github-devloop:implementing:v1 proposal="' .. tostring(proposal_id)
    .. '" dedup="' .. tostring(dedup_key)
    .. '" branch="' .. tostring(branch)
    .. '" head_sha="' .. tostring(head_sha)
    .. '" base_branch="' .. tostring(base_branch)
    .. '" base_sha="' .. tostring(base_sha)
    .. '" -->'
end

function M.pr_link_marker(proposal_id, pr_number, branch, impl_version, base_branch)
  if not M._is_positive_pr_number(pr_number) then
    error("github-devloop: invalid pr number")
  end
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  if not M._is_git_ref_safe(base_branch) then
    error("github-devloop: invalid base branch")
  end
  return '<!-- fkst:github-devloop:pr-link:v1 proposal="' .. tostring(proposal_id)
    .. '" pr="' .. tostring(pr_number)
    .. '" branch="' .. tostring(branch)
    .. '" impl_version="' .. tostring(impl_version)
    .. '" base_branch="' .. tostring(base_branch)
    .. '" -->'
end

function M.pr_link_marker_template(proposal_id, branch, impl_version, base_branch)
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  if not M._is_git_ref_safe(base_branch) then
    error("github-devloop: invalid base branch")
  end
  return '<!-- fkst:github-devloop:pr-link:v1 proposal="' .. tostring(proposal_id)
    .. '" pr="{{pr_number}}"'
    .. ' branch="' .. tostring(branch)
    .. '" impl_version="' .. tostring(impl_version)
    .. '" base_branch="' .. tostring(base_branch)
    .. '" -->'
end

function M.pr_origin_marker(proposal_id, issue_number, branch, impl_version, base_branch)
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  if not M._is_git_ref_safe(base_branch) then
    error("github-devloop: invalid base branch")
  end
  return '<!-- fkst:github-devloop:pr-origin:v1 proposal="' .. tostring(proposal_id)
    .. '" issue="' .. tostring(issue_number)
    .. '" branch="' .. tostring(branch)
    .. '" impl_version="' .. tostring(impl_version)
    .. '" base_branch="' .. tostring(base_branch)
    .. '" -->'
end

local max_digest_len = 64
local max_round = 100000

local function normalize_framing(value)
  return tostring(value or ""):gsub("%s+", " "):gsub("^%s+", ""):gsub("%s+$", "")
end

local function attr(marker, name)
  return marker:match(name .. '="([^"]*)"')
end

local function is_digest(value)
  return type(value) == "string" and value ~= "" and #value <= max_digest_len and value:find("%c") == nil
end

local function valid_round(value)
  local n = tonumber(value)
  if n == nil or n < 0 or n ~= math.floor(n) or n > max_round then
    return nil
  end
  return n
end

function M.review_reject_framing_digest(framing)
  local normalized = normalize_framing(framing)
  if #normalized > M._max_framing_len then
    normalized = normalized:sub(1, M._max_framing_len)
  end
  return "rf-" .. #normalized .. "-" .. M._decimal_checksum(normalized)
end

function M.review_result_marker(review_proposal_id, issue_proposal_id, decision, dedup_key, framing_digest, fix_round)
  if decision ~= "approve" and decision ~= "reject" then
    error("github-devloop: invalid review decision")
  end
  local framing_field = ""
  if decision == "reject" then
    if not is_digest(framing_digest) then
      error("github-devloop: invalid review reject framing digest")
    end
    framing_field = '" framing_digest="' .. tostring(framing_digest)
    if fix_round ~= nil then
      local n = valid_round(fix_round)
      if n == nil then
        error("github-devloop: invalid review reject fix round")
      end
      framing_field = framing_field .. '" fix_round="' .. tostring(n)
    end
  end
  return '<!-- fkst:github-devloop:review-result:v1 proposal="' .. tostring(review_proposal_id)
    .. '" issue_proposal="' .. tostring(issue_proposal_id)
    .. '" decision="' .. tostring(decision)
    .. '" dedup="' .. tostring(dedup_key)
    .. framing_field
    .. '" -->'
end

function M.merge_ready_marker(issue_proposal_id, pr_number, version, review_proposal_id, review_dedup_key, head_sha)
  if not M._is_positive_pr_number(pr_number) then
    error("github-devloop: invalid merge-ready pr number")
  end
  if not M._is_git_sha(head_sha) then
    error("github-devloop: invalid merge-ready head sha")
  end
  if not M._is_bounded_string(version, M._max_dedup_len)
    or not M._is_bounded_string(review_proposal_id, M._max_key_len)
    or not M._is_bounded_string(review_dedup_key, M._max_dedup_len) then
    error("github-devloop: invalid merge-ready marker")
  end
  return '<!-- fkst:github-devloop:merge-ready:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" pr="' .. tostring(pr_number)
    .. '" version="' .. tostring(version)
    .. '" review_proposal="' .. tostring(review_proposal_id)
    .. '" review_dedup="' .. tostring(review_dedup_key)
    .. '" head_sha="' .. tostring(head_sha)
    .. '" -->'
end

function M.merged_marker(issue_proposal_id, pr_number, version, head_sha)
  if not M._is_positive_pr_number(pr_number) or not M._is_git_sha(head_sha) then
    error("github-devloop: invalid merged marker")
  end
  return '<!-- fkst:github-devloop:merged:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" pr="' .. tostring(pr_number)
    .. '" version="' .. tostring(version)
    .. '" head_sha="' .. tostring(head_sha)
    .. '" -->'
end

function M.merging_marker(issue_proposal_id, pr_number, version, head_sha)
  if not M._is_positive_pr_number(pr_number) or not M._is_git_sha(head_sha) then
    error("github-devloop: invalid merging marker")
  end
  return '<!-- fkst:github-devloop:merging:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" pr="' .. tostring(pr_number)
    .. '" version="' .. tostring(version)
    .. '" head_sha="' .. tostring(head_sha)
    .. '" -->'
end

function M.intake_decision_marker(issue_proposal_id, decision, dedup_key)
  if decision ~= "enable" and decision ~= "decline" then
    error("github-devloop: invalid intake decision")
  end
  if not M._is_bounded_string(dedup_key, M._max_dedup_len) then
    error("github-devloop: invalid intake dedup")
  end
  return '<!-- fkst:github-devloop:intake-decision:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" decision="' .. tostring(decision)
    .. '" dedup="' .. tostring(dedup_key)
    .. '" -->'
end

function M.intake_decision_fact(comments, issue_proposal_id)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:intake%-decision:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local marker_issue = marker:match('proposal="([^"]+)"')
      local decision = marker:match('decision="([^"]+)"')
      local dedup = marker:match('dedup="([^"]*)"')
      if marker_issue == tostring(issue_proposal_id)
        and (decision == "enable" or decision == "decline")
        and M._is_bounded_string(dedup, M._max_dedup_len) then
        return {
          proposal_id = marker_issue,
          decision = decision,
          dedup_key = dedup,
          comment_created_at = M._comment_created_at(comment),
        }
      end
    end
  end
  return nil
end

function M.has_intake_decision_marker(comments, issue_proposal_id)
  return M.intake_decision_fact(comments, issue_proposal_id) ~= nil
end

function M.review_reject_fact(comments, issue_proposal_id, issue_version)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:review%-result:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local review_proposal = marker:match('proposal="([^"]+)"')
      local marker_issue = marker:match('issue_proposal="([^"]+)"')
      local decision = marker:match('decision="([^"]+)"')
      local review_dedup = marker:match('dedup="([^"]*)"')
      local _, _, review_version, reviewed_head_sha = M.parse_pr_review_proposal_id(review_proposal)
      if marker_issue == tostring(issue_proposal_id)
        and decision == "reject"
        and review_version == M.safe_version_segment(M._strip_latest_fix_version_suffix(issue_version))
        and M._is_bounded_string(review_dedup, M._max_dedup_len)
        and M._is_git_sha(reviewed_head_sha) then
        return {
          review_proposal_id = review_proposal,
          review_dedup_key = review_dedup,
          reviewed_head_sha = reviewed_head_sha,
          review_reason = M._comment_body(comment),
        }
      end
    end
  end
  return nil
end

function M.recent_review_reject_framing_digests(comments, issue_proposal_id, limit)
  local cap = tonumber(limit) or M.fix_stall_rounds()
  if cap <= 0 or type(comments) ~= "table" then
    return {}
  end

  local facts = {}
  local marker_pattern = "<!%-%- fkst:github%-devloop:review%-result:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local review_proposal = attr(marker, "proposal")
      local marker_issue = attr(marker, "issue_proposal")
      local decision = attr(marker, "decision")
      local framing_digest = attr(marker, "framing_digest")
      local _, _, review_version = M.parse_pr_review_proposal_id(review_proposal)
      local fix_round = valid_round(attr(marker, "fix_round"))
      if fix_round == nil then
        fix_round = M.version_fix_round(review_version)
      end
      if marker_issue == tostring(issue_proposal_id)
        and decision == "reject"
        and is_digest(framing_digest)
        and review_version ~= nil then
        table.insert(facts, {
          round = fix_round,
          digest = framing_digest,
          comment_created_at = M._comment_created_at(comment) or "",
        })
      end
    end
  end

  table.sort(facts, function(a, b)
    if a.round == b.round then
      return tostring(a.comment_created_at) > tostring(b.comment_created_at)
    end
    return a.round > b.round
  end)

  local digests = {}
  for _, fact in ipairs(facts) do
    if #digests >= cap then
      break
    end
    table.insert(digests, fact.digest)
  end
  return digests
end

function M.is_fix_framing_true_stall(comments, issue_proposal_id, current_framing)
  local expected_count = M.fix_stall_rounds()
  local digests = M.recent_review_reject_framing_digests(
    comments,
    issue_proposal_id,
    expected_count - 1
  )
  table.insert(digests, 1, M.review_reject_framing_digest(current_framing))
  if #digests < expected_count then
    return false
  end

  local current = digests[1]
  if current == nil or current == "" then
    return false
  end
  for i = 2, expected_count do
    if digests[i] ~= current then
      return false
    end
  end
  return true
end

function M.review_meta_fix_fact(comments, issue_proposal_id, issue_version)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:review%-meta:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local marker_issue = marker:match('proposal="([^"]+)"')
      local marker_dedup = marker:match('dedup="([^"]*)"')
      local action = marker:match('action="([^"]+)"')
      local version = marker:match('version="([^"]*)"')
      if marker_issue == tostring(issue_proposal_id)
        and marker_dedup ~= nil
        and action == "fix"
        and version == tostring(issue_version) then
        local review_proposal = marker_dedup:match("^consensus:([^/].-)/review")
        local _, _, _, reviewed_head_sha = M.parse_pr_review_proposal_id(review_proposal)
        return {
          review_proposal_id = review_proposal,
          review_dedup_key = marker_dedup,
          reviewed_head_sha = reviewed_head_sha,
          review_reason = M._comment_body(comment),
        }
      end
    end
  end
  return nil
end

function M.merge_gate_fix_fact(comments, issue_proposal_id, issue_version)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:merge%-gate:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local marker_issue = marker:match('proposal="([^"]+)"')
      local marker_version = marker:match('version="([^"]*)"')
      local marker_review_proposal = marker:match('review_proposal="([^"]+)"')
      local marker_review_dedup = marker:match('review_dedup="([^"]*)"')
      local marker_head_sha = marker:match('head_sha="([^"]+)"')
      if marker_issue == tostring(issue_proposal_id)
        and marker_version == tostring(issue_version)
        and M._is_bounded_string(marker_review_proposal, M._max_key_len)
        and M._is_bounded_string(marker_review_dedup, M._max_dedup_len)
        and M._is_git_sha(marker_head_sha) then
        return {
          review_proposal_id = marker_review_proposal,
          review_dedup_key = marker_review_dedup,
          reviewed_head_sha = marker_head_sha,
          review_reason = M._comment_body(comment),
        }
      end
    end
  end
  return nil
end

function M.merge_ready_fact(comments, issue_proposal_id, issue_version, pr_number)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:merge%-ready:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local marker_issue = marker:match('proposal="([^"]+)"')
      local marker_pr = marker:match('pr="([^"]+)"')
      local marker_version = marker:match('version="([^"]*)"')
      local marker_review_proposal = marker:match('review_proposal="([^"]+)"')
      local marker_review_dedup = marker:match('review_dedup="([^"]*)"')
      local marker_head_sha = marker:match('head_sha="([^"]+)"')
      if marker_issue == tostring(issue_proposal_id)
        and (pr_number == nil or tostring(marker_pr) == tostring(pr_number))
        and tostring(marker_version) == tostring(issue_version)
        and M._is_bounded_string(marker_review_proposal, M._max_key_len)
        and M._is_bounded_string(marker_review_dedup, M._max_dedup_len)
        and M._is_git_sha(marker_head_sha) then
        return {
          proposal_id = marker_issue,
          pr_number = tonumber(marker_pr),
          version = marker_version,
          review_proposal_id = marker_review_proposal,
          review_dedup_key = marker_review_dedup,
          head_sha = marker_head_sha,
          comment_created_at = M._comment_created_at(comment),
        }
      end
    end
  end
  return nil
end

function M.review_result_approval_matches_event(comments, merge_ready)
  if type(comments) ~= "table" or type(merge_ready) ~= "table" then
    return false, "missing-review-result-approve"
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:review%-result:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local review_proposal = marker:match('proposal="([^"]+)"')
      local issue_proposal = marker:match('issue_proposal="([^"]+)"')
      local decision = marker:match('decision="([^"]+)"')
      local review_dedup = marker:match('dedup="([^"]*)"')
      local _, review_pr_number, review_version, reviewed_head_sha = M.parse_pr_review_proposal_id(review_proposal)
      if tostring(review_proposal or "") == tostring(merge_ready.review_proposal_id or "")
        and tostring(issue_proposal or "") == tostring(merge_ready.proposal_id or "")
        and decision == "approve"
        and tostring(review_dedup or "") == tostring(merge_ready.review_dedup_key or "")
        and tostring(review_pr_number or "") == tostring(merge_ready.pr_number or "")
        and tostring(reviewed_head_sha or "") == tostring(merge_ready.reviewed_head_sha or "")
        and tostring(review_version or "") == M.safe_version_segment(merge_ready.version) then
        return true, "review-result-approve"
      end
    end
  end
  return false, "missing-review-result-approve"
end

local function review_proposal_version_matches_merge_ready(review_version, merge_ready_version, review_dedup_key)
  local merge_text = tostring(merge_ready_version or "")
  if tostring(review_version or "") == M.safe_version_segment(merge_text) then
    return true
  end
  local base = merge_text:match("^(.-)/review%-loop/%d+")
  if base == nil then
    return false
  end
  return tostring(review_dedup_key or ""):find("review%-meta", 1) ~= nil
    and tostring(review_version or "") == M.safe_version_segment(base)
    and merge_text:find("/review%-meta%-action/", 1) ~= nil
end

function M.merge_ready_approval_matches_event(fact, merge_ready)
  if type(fact) ~= "table" or type(merge_ready) ~= "table" then
    return false, "missing-merge-ready-approval"
  end
  if tostring(fact.proposal_id or "") ~= tostring(merge_ready.proposal_id or "")
    or tostring(fact.pr_number or "") ~= tostring(merge_ready.pr_number or "")
    or tostring(fact.version or "") ~= tostring(merge_ready.version or "")
    or tostring(fact.review_proposal_id or "") ~= tostring(merge_ready.review_proposal_id or "")
    or tostring(fact.review_dedup_key or "") ~= tostring(merge_ready.review_dedup_key or "")
    or tostring(fact.head_sha or "") ~= tostring(merge_ready.reviewed_head_sha or "") then
    return false, "merge-ready-approval-mismatch"
  end

  local issue_repo = M.parse_proposal_id(merge_ready.proposal_id)
  local review_repo, review_pr_number, review_version, review_head_sha = M.parse_pr_review_proposal_id(fact.review_proposal_id)
  local expected_review_repo = issue_repo and M.safe_pr_review_repo_segment(issue_repo) or nil
  if review_repo == nil
    or tostring(review_repo) ~= tostring(expected_review_repo or "")
    or tostring(review_pr_number) ~= tostring(merge_ready.pr_number or "")
    or tostring(review_head_sha) ~= tostring(merge_ready.reviewed_head_sha or "") then
    return false, "merge-ready-review-proposal-mismatch"
  end
  if not review_proposal_version_matches_merge_ready(review_version, merge_ready.version, merge_ready.review_dedup_key) then
    return false, "merge-ready-review-proposal-version-mismatch"
  end

  return true, "merge-ready-approval"
end

function M.merging_fact(comments, issue_proposal_id, pr_number, version, head_sha)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:merging:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local marker_issue = marker:match('proposal="([^"]+)"')
      local marker_pr = marker:match('pr="([^"]+)"')
      local marker_version = marker:match('version="([^"]*)"')
      local marker_head_sha = marker:match('head_sha="([^"]+)"')
      if marker_issue == tostring(issue_proposal_id)
        and tostring(marker_pr) == tostring(pr_number)
        and tostring(marker_version) == tostring(version)
        and tostring(marker_head_sha) == tostring(head_sha)
        and M._is_git_sha(marker_head_sha) then
        return {
          proposal_id = marker_issue,
          pr_number = tonumber(marker_pr),
          version = marker_version,
          head_sha = marker_head_sha,
          comment_created_at = M._comment_created_at(comment),
        }
      end
    end
  end
  return nil
end

function M.has_merged_marker(comments, issue_proposal_id, pr_number, version, head_sha)
  if type(comments) ~= "table" then
    return false
  end
  local marker = M.merged_marker(issue_proposal_id, pr_number, version, head_sha)
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    if M._comment_body(comment):find(marker, 1, true) ~= nil then
      return true
    end
  end
  return false
end

function M.impl_failure_marker(proposal_id, dedup_key, reason)
  local safe_reason = M.sanitize_key(reason or "failed"):gsub("/", "-")
  return '<!-- fkst:github-devloop:impl-failure:v1 proposal="' .. tostring(proposal_id)
    .. '" reason="' .. safe_reason
    .. '" dedup="' .. tostring(dedup_key)
    .. '" -->'
end

function M.has_review_result_marker(comments, review_proposal_id, issue_proposal_id, decision, dedup_key)
  if type(comments) ~= "table" then
    return false
  end
  local needle = M.review_result_marker(review_proposal_id, issue_proposal_id, decision, dedup_key)
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    if M._comment_body(comment):find(needle, 1, true) ~= nil then
      return true
    end
  end
  return false
end

function M.has_review_meta_marker(comments, issue_proposal_id, dedup_key)
  if type(comments) ~= "table" then
    return false
  end

  local marker_pattern = "<!%-%- fkst:github%-devloop:review%-meta:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local marker_dedup = marker:match('dedup="([^"]*)"')
      if marker_proposal == tostring(issue_proposal_id) and marker_dedup == tostring(dedup_key) then
        return true
      end
    end
  end
  return false
end

function M.has_fix_marker(comments, issue_proposal_id, review_proposal_id, review_dedup_key, old_head_sha, new_head_sha)
  if type(comments) ~= "table" then
    return false
  end
  local needle = M.fix_marker(issue_proposal_id, review_proposal_id, review_dedup_key, old_head_sha, new_head_sha)
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    if M._comment_body(comment):find(needle, 1, true) ~= nil then
      return true
    end
  end
  return false
end

function M.has_any_review_result_marker(comments, review_proposal_id, issue_proposal_id)
  if type(comments) ~= "table" then
    return false
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:review%-result:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      if marker:match('proposal="([^"]+)"') == tostring(review_proposal_id)
        and marker:match('issue_proposal="([^"]+)"') == tostring(issue_proposal_id) then
        return true
      end
    end
  end
  return false
end

local function has_versioned_marker(comments, marker)
  if type(comments) ~= "table" then
    return false
  end
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    if M._comment_body(comment):find(marker, 1, true) ~= nil then
      return true
    end
  end
  return false
end

function M.has_implementing_marker(comments, proposal_id, dedup_key)
  if type(comments) ~= "table" then
    return false
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:implementing:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      if marker:match('proposal="([^"]+)"') == tostring(proposal_id)
        and marker:match('dedup="([^"]*)"') == tostring(dedup_key) then
        return true
      end
    end
  end
  return false
end

function M.is_safe_branch(branch)
  return M._is_git_ref_safe(branch)
end

function M.is_devloop_issue_branch(branch)
  return type(branch) == "string"
    and M._is_git_ref_safe(branch)
    and branch:find("^devloop/issue/[^/]+/.+/.+") ~= nil
end

function M.is_safe_head_sha(head_sha)
  return M._is_git_sha(head_sha)
end

function M.is_safe_pr_number(pr_number)
  return M._is_positive_pr_number(pr_number)
end

function M.is_same_repo_pr_head(pr, repo)
  if type(pr) ~= "table" then
    return false
  end
  if pr.is_cross_repository == true then
    return false
  end
  if pr.head_repository == nil then
    return false
  end
  return tostring(pr.head_repository):lower() == tostring(repo):lower()
end

function M.implementing_fact(comments, proposal_id, dedup_key)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:implementing:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local marker_dedup = marker:match('dedup="([^"]*)"')
      local marker_branch = marker:match('branch="([^"]+)"')
      local marker_head_sha = marker:match('head_sha="([^"]+)"')
      local marker_base_branch = marker:match('base_branch="([^"]+)"')
      local marker_base_sha = marker:match('base_sha="([^"]+)"')
      if marker_proposal == proposal_id
        and marker_dedup == tostring(dedup_key)
        and M._is_git_ref_safe(marker_branch)
        and M._is_git_sha(marker_head_sha)
        and M._is_git_ref_safe(marker_base_branch)
        and M._is_git_sha(marker_base_sha) then
        return {
          proposal_id = marker_proposal,
          dedup_key = marker_dedup,
          branch = marker_branch,
          head_sha = marker_head_sha,
          base_branch = marker_base_branch,
          base_sha = marker_base_sha,
        }
      end
    end
  end
  return nil
end

function M.pr_link_fact(comments, proposal_id)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:pr%-link:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local marker_pr = marker:match('pr="([^"]+)"')
      local marker_branch = marker:match('branch="([^"]+)"')
      local marker_impl_version = marker:match('impl_version="([^"]*)"')
      local marker_base_branch = marker:match('base_branch="([^"]+)"')
      if marker_proposal == proposal_id
        and M._is_positive_pr_number(marker_pr)
        and M._is_git_ref_safe(marker_branch)
        and M._is_bounded_string(marker_impl_version, M._max_dedup_len)
        and M._is_git_ref_safe(marker_base_branch) then
        return {
          proposal_id = marker_proposal,
          pr_number = tonumber(marker_pr),
          branch = marker_branch,
          impl_version = marker_impl_version,
          base_branch = marker_base_branch,
        }
      end
    end
  end
  return nil
end

function M.pr_origin_fact(comments)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:pr%-origin:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local marker_issue = marker:match('issue="([^"]+)"')
      local marker_branch = marker:match('branch="([^"]+)"')
      local marker_impl_version = marker:match('impl_version="([^"]*)"')
      local marker_base_branch = marker:match('base_branch="([^"]+)"')
      local repo, issue_number = M.parse_proposal_id(marker_proposal)
      if repo ~= nil
        and marker_issue == issue_number
        and M._is_git_ref_safe(marker_branch)
        and M._is_bounded_string(marker_impl_version, M._max_dedup_len)
        and M._is_git_ref_safe(marker_base_branch) then
        return {
          proposal_id = marker_proposal,
          repo = repo,
          issue_number = issue_number,
          branch = marker_branch,
          impl_version = marker_impl_version,
          base_branch = marker_base_branch,
        }
      end
    end
  end
  return nil
end

function M.has_impl_failure_marker(comments, proposal_id, dedup_key)
  if type(comments) ~= "table" then
    return false
  end

  local marker_pattern = "<!%-%- fkst:github%-devloop:impl%-failure:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local marker_dedup = marker:match('dedup="([^"]*)"')
      if marker_proposal == proposal_id and marker_dedup == tostring(dedup_key) then
        return true
      end
    end
  end
  return false
end

function M.has_implementation_fact_marker(comments, proposal_id, dedup_key)
  return M.has_implementing_marker(comments, proposal_id, dedup_key)
    or M.has_impl_failure_marker(comments, proposal_id, dedup_key)
end

function M.result_marker(proposal_id, decision, dedup_key)
  if decision ~= "approve" and decision ~= "reject" then
    error("github-devloop: invalid decision")
  end
  return '<!-- fkst:github-devloop:result:v1 proposal="' .. tostring(proposal_id)
    .. '" decision="' .. decision
    .. '" dedup="' .. tostring(dedup_key)
    .. '" -->'
end
end

return S
