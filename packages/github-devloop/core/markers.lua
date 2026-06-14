local S = {}

local max_round = 100000
local max_attr_len = 240

local function valid_round(value)
  local n = tonumber(value)
  if n == nil or n < 0 or n ~= math.floor(n) or n > max_round then
    return nil
  end
  return n
end

local function marker_attr(marker, name)
  return marker:match(name .. '="([^"]*)"')
end

local function safe_marker_attr(M, value, limit)
  local text = tostring(value or "")
  text = text:gsub("<!%-%- fkst:[^\n]*%-%->", " ")
  text = text:gsub("&lt;!%-%- fkst:[^\n]*%-%-&gt;", " ")
  text = text:gsub("%c", " "):gsub('"', "'"):gsub("[<>]", ""):gsub("%s+", " ")
  text = text:gsub("^%s+", ""):gsub("%s+$", "")
  local cap = limit or max_attr_len
  if #text > cap then
    text = M.truncate_utf8(text, cap)
  end
  return text
end

local function decode_marker_attr(value)
  if type(value) ~= "string" or value == "" then
    return nil
  end
  if value:find("%c") ~= nil or value:find("[<>]") ~= nil or value:find('"', 1, true) ~= nil then
    return nil
  end
  return value
end

local function review_result_fact_from_marker(M, marker, comment, issue_proposal_id, issue_version, expected_decision)
  local review_proposal = marker_attr(marker, "proposal")
  local marker_issue = marker_attr(marker, "issue_proposal")
  local decision = marker_attr(marker, "decision")
  local review_dedup = marker_attr(marker, "dedup")
  local _, _, review_version, reviewed_head_sha = M.parse_pr_review_proposal_id(review_proposal)
  local expected_dedup = review_proposal ~= nil and ("consensus:" .. tostring(review_proposal) .. "/review") or nil
  if marker_issue == tostring(issue_proposal_id)
    and (expected_decision == nil or decision == expected_decision)
    and (decision == "approve" or decision == "reject")
    and review_version == M.safe_version_segment(M._strip_latest_fix_version_suffix(issue_version))
    and review_dedup == expected_dedup
    and M._is_bounded_string(review_dedup, M._max_dedup_len)
    and M._is_git_sha(reviewed_head_sha) then
    local fact = {
      review_proposal_id = review_proposal,
      review_dedup_key = review_dedup,
      reviewed_head_sha = reviewed_head_sha,
      decision = decision,
      review_reason = M._comment_body(comment),
      comment_created_at = M._comment_created_at(comment),
    }
    if decision == "reject" then
      local marker_fix_round = valid_round(marker_attr(marker, "fix_round"))
      if marker_fix_round == nil or marker_fix_round ~= M.version_fix_round(issue_version) then
        return nil
      end
      local gap = decode_marker_attr(marker_attr(marker, "gap"))
      if gap == nil or not M._is_bounded_string(gap, M._max_blocking_gap_len) then
        return nil
      end
      fact.blocking_gap = gap
      fact.fix_round = marker_fix_round
    end
    return fact
  end
  return nil
end

function S.install(M)
function M.review_meta_marker(issue_proposal_id, dedup_key, action, version, blocking_gap, reason)
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
  if action == "fix" then
    local gap = safe_marker_attr(M, blocking_gap, M._max_blocking_gap_len)
    if gap == "" or not M._is_bounded_string(gap, M._max_blocking_gap_len) then
      error("github-devloop: invalid review-meta gap")
    end
    fields = fields .. '" gap="' .. gap
  elseif action == "spec-amendment" then
    fields = fields .. '" reason="blocked-pending-spec'
  end
  return '<!-- fkst:github-devloop:review-meta:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" dedup="' .. tostring(dedup_key)
    .. fields
    .. '" -->'
end

function M.fix_reflection_marker(issue_proposal_id, dedup_key, verdict, version, fix_round)
  if verdict ~= "checkpoint" and verdict ~= "continue" and verdict ~= "spec-gap" then
    error("github-devloop: invalid fix reflection verdict")
  end
  local n = valid_round(fix_round)
  if n == nil then
    error("github-devloop: invalid fix reflection round")
  end
  local version_field = ""
  if version ~= nil then
    version_field = '" version="' .. tostring(version)
  end
  return '<!-- fkst:github-devloop:fix-reflection:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" dedup="' .. tostring(dedup_key)
    .. '" verdict="' .. tostring(verdict)
    .. version_field
    .. '" fix_round="' .. tostring(n)
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

function M.merge_gate_marker(issue_proposal_id, pr_number, version, review_proposal_id, review_dedup_key, head_sha, gate_baseline_sha, reason, predecessor_set)
  if not M._is_positive_pr_number(pr_number) or not M._is_git_sha(head_sha) then
    error("github-devloop: invalid merge-gate marker")
  end
  local baseline_field = ""
  if gate_baseline_sha ~= nil then
    if not M._is_git_sha(gate_baseline_sha) then
      error("github-devloop: invalid merge-gate marker")
    end
    baseline_field = '" gate_baseline_sha="' .. tostring(gate_baseline_sha)
  end
  local predecessor_field = ""
  if predecessor_set ~= nil then
    if not M._is_path_safe_key(predecessor_set, M._max_dedup_len) then
      error("github-devloop: invalid merge-gate predecessor set")
    end
    predecessor_field = '" predecessor_set="' .. tostring(predecessor_set)
  end
  return '<!-- fkst:github-devloop:merge-gate:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" pr="' .. tostring(pr_number)
    .. '" version="' .. tostring(version)
    .. '" review_proposal="' .. tostring(review_proposal_id)
    .. '" review_dedup="' .. tostring(review_dedup_key)
    .. '" head_sha="' .. tostring(head_sha)
    .. baseline_field
    .. predecessor_field
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

function M.review_result_marker(review_proposal_id, issue_proposal_id, decision, dedup_key, fix_round, blocking_gap)
  if decision ~= "approve" and decision ~= "reject" then
    error("github-devloop: invalid review decision")
  end
  local fix_round_field = ""
  local gap_field = ""
  if decision == "reject" then
    if fix_round ~= nil then
      local n = valid_round(fix_round)
      if n == nil then
        error("github-devloop: invalid review reject fix round")
      end
      fix_round_field = '" fix_round="' .. tostring(n)
    end
    local gap = safe_marker_attr(M, blocking_gap, M._max_blocking_gap_len)
    if gap == "" or not M._is_bounded_string(gap, M._max_blocking_gap_len) then
      error("github-devloop: invalid review reject gap")
    end
    gap_field = '" gap="' .. gap
  end
  return '<!-- fkst:github-devloop:review-result:v1 proposal="' .. tostring(review_proposal_id)
    .. '" issue_proposal="' .. tostring(issue_proposal_id)
    .. '" decision="' .. tostring(decision)
    .. '" dedup="' .. tostring(dedup_key)
    .. fix_round_field
    .. gap_field
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

function M.review_carry_over_marker(issue_proposal_id, version, old_review_proposal_id, old_review_dedup_key, approved_head_sha, new_review_proposal_id, new_review_dedup_key, new_head_sha, base_head_sha)
  if not M._is_git_sha(approved_head_sha)
    or not M._is_git_sha(new_head_sha)
    or not M._is_git_sha(base_head_sha) then
    error("github-devloop: invalid review carry-over marker")
  end
  if not M._is_bounded_string(version, M._max_dedup_len)
    or not M._is_bounded_string(old_review_proposal_id, M._max_key_len)
    or not M._is_bounded_string(old_review_dedup_key, M._max_dedup_len)
    or not M._is_bounded_string(new_review_proposal_id, M._max_key_len)
    or not M._is_bounded_string(new_review_dedup_key, M._max_dedup_len) then
    error("github-devloop: invalid review carry-over marker")
  end
  return '<!-- fkst:github-devloop:review-carry-over:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" version="' .. tostring(version)
    .. '" old_review_proposal="' .. tostring(old_review_proposal_id)
    .. '" old_review_dedup="' .. tostring(old_review_dedup_key)
    .. '" approved_head_sha="' .. tostring(approved_head_sha)
    .. '" new_review_proposal="' .. tostring(new_review_proposal_id)
    .. '" new_review_dedup="' .. tostring(new_review_dedup_key)
    .. '" new_head_sha="' .. tostring(new_head_sha)
    .. '" base_head_sha="' .. tostring(base_head_sha)
    .. '" proof="merge-tree-empty-delta" -->'
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

function M.intake_decision_marker(issue_proposal_id, decision, dedup_key, service_class)
  if decision ~= "enable" and decision ~= "track" and decision ~= "decline" and decision ~= "escalate-to-class" then
    error("github-devloop: invalid intake decision")
  end
  if not M._is_bounded_string(dedup_key, M._max_dedup_len) then
    error("github-devloop: invalid intake dedup")
  end
  if not M.is_intake_service_class(service_class) then
    error("github-devloop: invalid intake service class")
  end
  local normalized_class = M.normalize_intake_service_class(service_class)
  return '<!-- fkst:github-devloop:intake-decision:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" decision="' .. tostring(decision)
    .. '" class="' .. normalized_class
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
      local service_class = marker:match('class="([^"]+)"')
      local dedup = marker:match('dedup="([^"]*)"')
      if marker_issue == tostring(issue_proposal_id)
        and (decision == "enable" or decision == "track" or decision == "decline" or decision == "escalate-to-class")
        and M.is_intake_service_class(service_class)
        and M._is_bounded_string(dedup, M._max_dedup_len) then
        return {
          proposal_id = marker_issue,
          decision = decision,
          service_class = M.normalize_intake_service_class(service_class),
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
      local fact = review_result_fact_from_marker(M, marker, comment, issue_proposal_id, issue_version, "reject")
      if fact ~= nil then
        return fact
      end
    end
  end
  return nil
end

local function bounded_marker_line(M, value, limit)
  local text = tostring(value or ""):gsub("%c", " "):gsub("%s+", " ")
  text = text:gsub("^%s+", ""):gsub("%s+$", "")
  if text == "" then
    return nil
  end
  local cap = limit or M._max_blocking_gap_len
  if #text > cap then
    text = M.truncate_utf8(text, cap)
  end
  return text
end

local function highest_state_fix_round(M, body, issue_proposal_id)
  local highest = nil
  local marker_pattern = "<!%-%- fkst:github%-devloop:state:v1.-%-%->"
  for marker in tostring(body or ""):gmatch(marker_pattern) do
    if marker_attr(marker, "proposal") == tostring(issue_proposal_id) then
      local round = M.version_fix_round(marker_attr(marker, "version"))
      if highest == nil or round > highest then
        highest = round
      end
    end
  end
  return highest
end

function M.review_prior_round_ledger(comments, issue_proposal_id, issue_version)
  if type(comments) ~= "table" then
    return nil
  end
  local latest_reject = nil
  local latest_fix = nil
  local marker_pattern = "<!%-%- fkst:github%-devloop:review%-result:v1.-%-%->"
  local rejected_fix_version = M._strip_latest_fix_version_suffix(issue_version)
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    local body = M._comment_body(comment)
    for marker in body:gmatch(marker_pattern) do
      local fact = review_result_fact_from_marker(M, marker, comment, issue_proposal_id, rejected_fix_version, "reject")
      if fact ~= nil and (latest_reject == nil or fact.fix_round > latest_reject.fix_round) then
        latest_reject = {
          gap = fact.blocking_gap,
          fix_round = fact.fix_round,
          created_at = M._comment_created_at(comment),
        }
      end
    end
    local fix_summary = body:match("\nFix%-round summary:%s*([^\n]+)") or body:match("^Fix%-round summary:%s*([^\n]+)")
    fix_summary = bounded_marker_line(M, fix_summary, M._max_review_ledger_len)
    local fix_summary_round = highest_state_fix_round(M, body, issue_proposal_id)
    if fix_summary ~= nil
      and fix_summary_round ~= nil
      and (latest_fix == nil or fix_summary_round > latest_fix.fix_round) then
      latest_fix = {
        summary = fix_summary,
        fix_round = fix_summary_round,
        created_at = M._comment_created_at(comment),
      }
    end
  end
  if latest_reject == nil then
    return nil
  end
  local lines = {
    "Last named blocking gap: " .. latest_reject.gap,
  }
  if latest_fix ~= nil then
    table.insert(lines, "Latest fix-round summary: " .. latest_fix.summary)
  end
  local ledger = table.concat(lines, "\n")
  if #ledger > M._max_review_ledger_len then
    ledger = M.truncate_utf8(ledger, M._max_review_ledger_len)
  end
  return M.neutralize_untrusted_prompt_text(ledger)
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
      local gap = decode_marker_attr(marker_attr(marker, "gap"))
      if marker_issue == tostring(issue_proposal_id)
        and marker_dedup ~= nil
        and action == "fix"
        and version == tostring(issue_version)
        and M._is_bounded_string(gap, M._max_blocking_gap_len) then
        local review_proposal = marker_dedup:match("^consensus:([^/].-)/review")
        local _, _, _, reviewed_head_sha = M.parse_pr_review_proposal_id(review_proposal)
        return {
          review_proposal_id = review_proposal,
          review_dedup_key = marker_dedup,
          reviewed_head_sha = reviewed_head_sha,
          review_reason = M._comment_body(comment),
          blocking_gap = gap,
        }
      end
    end
  end
  return nil
end

local function merge_gate_fix_fact_matches_bindings(fact, opts)
  if type(opts) ~= "table" then
    return true
  end
  local baseline_bound = opts.match_gate_baseline_sha == true or opts.gate_baseline_sha ~= nil
  return (opts.review_proposal_id == nil or fact.review_proposal_id == tostring(opts.review_proposal_id))
    and (opts.review_dedup_key == nil or fact.review_dedup_key == tostring(opts.review_dedup_key))
    and (not baseline_bound
      or (opts.gate_baseline_sha ~= nil and fact.gate_baseline_sha == tostring(opts.gate_baseline_sha))
      or (opts.gate_baseline_sha == nil and fact.gate_baseline_sha == nil))
end
function M.merge_gate_fix_fact(comments, issue_proposal_id, issue_version, opts)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:merge%-gate:v1.-%-%->"
  local first_fact = nil
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local marker_issue = marker:match('proposal="([^"]+)"')
      local marker_version = marker:match('version="([^"]*)"')
      local marker_review_proposal = marker:match('review_proposal="([^"]+)"')
      local marker_review_dedup = marker:match('review_dedup="([^"]*)"')
      local marker_head_sha = marker:match('head_sha="([^"]+)"')
      local marker_gate_baseline_sha = marker:match('gate_baseline_sha="([^"]+)"')
      local marker_predecessor_set = marker:match('predecessor_set="([^"]+)"')
      local marker_reason = marker:match('reason="([^"]+)"')
      if marker_issue == tostring(issue_proposal_id)
        and marker_version == tostring(issue_version)
        and M._is_bounded_string(marker_review_proposal, M._max_key_len)
        and M._is_bounded_string(marker_review_dedup, M._max_dedup_len)
        and M._is_bounded_string(marker_reason, M._max_key_len)
        and M._is_git_sha(marker_head_sha)
        and (marker_gate_baseline_sha == nil or M._is_git_sha(marker_gate_baseline_sha))
        and (marker_predecessor_set == nil or M._is_path_safe_key(marker_predecessor_set, M._max_dedup_len)) then
        local fact = {
          review_proposal_id = marker_review_proposal,
          review_dedup_key = marker_review_dedup,
          reviewed_head_sha = marker_head_sha,
          gate_baseline_sha = marker_gate_baseline_sha,
          predecessor_set = marker_predecessor_set,
          reason = marker_reason,
          review_reason = M._comment_body(comment),
        }
        if first_fact == nil then
          first_fact = fact
        end
        if merge_gate_fix_fact_matches_bindings(fact, opts) then
          return fact
        end
      end
    end
  end
  if type(opts) == "table" then
    return nil
  end
  return first_fact
end

function M.merge_ready_fact(comments, issue_proposal_id, issue_version, pr_number, head_sha)
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
        and (head_sha == nil or tostring(marker_head_sha) == tostring(head_sha))
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

  local entity = M.parse_entity_proposal_id(merge_ready.proposal_id)
  local entity_repo = entity and entity.repo or nil
  local review_repo, review_pr_number, review_version, review_head_sha = M.parse_pr_review_proposal_id(fact.review_proposal_id)
  local expected_review_repo = entity_repo and M.safe_pr_review_repo_segment(entity_repo) or nil
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

function M.merged_fact(comments, issue_proposal_id, pr_number, version)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:merged:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local marker_issue = marker:match('proposal="([^"]+)"')
      local marker_pr = marker:match('pr="([^"]+)"')
      local marker_version = marker:match('version="([^"]*)"')
      local marker_head_sha = marker:match('head_sha="([^"]+)"')
      if marker_issue == tostring(issue_proposal_id)
        and tostring(marker_pr) == tostring(pr_number)
        and (version == nil or tostring(marker_version) == tostring(version))
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
  local fact = M.merged_fact(comments, issue_proposal_id, pr_number, version)
  return fact ~= nil and tostring(fact.head_sha) == tostring(head_sha)
end

function M.has_review_result_marker(comments, review_proposal_id, issue_proposal_id, decision, dedup_key)
  if type(comments) ~= "table" then
    return false
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:review%-result:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      if marker_attr(marker, "proposal") == tostring(review_proposal_id)
        and marker_attr(marker, "issue_proposal") == tostring(issue_proposal_id)
        and marker_attr(marker, "decision") == tostring(decision)
        and marker_attr(marker, "dedup") == tostring(dedup_key) then
        return true
      end
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

function M.orphan_reaped_marker(proposal_id, pr_number, reason)
  if not M._is_positive_pr_number(pr_number) then
    error("github-devloop: invalid orphan reaped pr number")
  end
  local safe_reason = M.sanitize_key(reason or "parent-terminal", false):gsub("/", "-")
  return '<!-- fkst:github-devloop:orphan-reaped:v1 proposal="' .. tostring(proposal_id)
    .. '" pr="' .. tostring(pr_number)
    .. '" reason="' .. tostring(safe_reason)
    .. '" -->'
end

function M.has_orphan_reaped_marker(comments, proposal_id, pr_number)
  if type(comments) ~= "table" then
    return false
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:orphan%-reaped:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      if marker:match('proposal="([^"]+)"') == tostring(proposal_id)
        and tostring(marker:match('pr="([^"]+)"')) == tostring(pr_number) then
        return true
      end
    end
  end
  return false
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
