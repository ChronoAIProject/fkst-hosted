local S = {}

function S.install(M)
local function bounded_framing(M, framing)
  if framing == nil then
    return nil
  end
  local value = tostring(framing)
  if #value > M._max_framing_len then
    value = M.truncate_utf8(value, M._max_framing_len)
  end
  return value
end

local function bounded_control_text(M, value, limit)
  if value == nil then
    return nil
  end
  local text = tostring(value):gsub("%c", " "):gsub("%s+", " ")
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

local function board_digest_issue_list_cmd(M, repo)
  return "gh issue list"
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --state open"
    .. " --limit 100"
    .. " --json number,title,labels"
end

local function board_digest_pr_list_cmd(M, repo)
  return "gh pr list"
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --state open"
    .. " --limit 100"
    .. " --json number,title,labels"
end

local function recent_closed_issue_list_cmd(M, repo)
  if type(M.gh_issue_list_recent_closed_cmd) == "function" then
    return M.gh_issue_list_recent_closed_cmd(repo, 30)
  end
  return "gh issue list"
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --state closed"
    .. " --limit 30"
    .. " --json number,title,closedAt,labels"
end

local function label_names(labels_json)
  local labels = {}
  for _, label in ipairs(labels_json or {}) do
    if type(label) == "table" and label.name ~= nil then
      table.insert(labels, tostring(label.name))
    elseif type(label) == "string" then
      table.insert(labels, label)
    end
  end
  return labels
end

local function parse_board_list(stdout)
  local decoded = json.decode(stdout or "[]")
  local items = {}
  if type(decoded) ~= "table" then
    return items
  end
  for _, item in ipairs(decoded) do
    if type(item) == "table" and tonumber(item.number) ~= nil then
      table.insert(items, {
        number = tonumber(item.number),
        title = tostring(item.title or ""),
        labels = label_names(item.labels),
      })
    end
  end
  return items
end

local function first_chars(M, value, limit)
  local text = tostring(value or ""):gsub("[%s]+", " ")
  if #text > limit then
    return M.truncate_utf8(text, limit)
  end
  return text
end

local function recurrence_label_digest(M, labels)
  local selected = {}
  for _, label in ipairs(labels or {}) do
    local text = tostring(label)
    if text:find("^error%-class:", 1) ~= nil
      or text:find("^fingerprint:", 1) ~= nil
      or text:find("^fkst%-dev:", 1) ~= nil then
      table.insert(selected, text)
    end
    if #selected >= 4 then
      break
    end
  end
  if #selected == 0 then
    return "labels=none"
  end
  return "labels=" .. first_chars(M, table.concat(selected, ","), 120)
end

local function state_label(M, labels)
  for _, label in ipairs(labels or {}) do
    local text = tostring(label)
    if M._state_labels[text] then
      return text
    end
  end
  return "open"
end

local function render_closed_issue_line(M, item)
  return "#" .. tostring(item.number)
    .. " [closed] "
    .. first_chars(M, item.title, 80)
    .. " (" .. recurrence_label_digest(M, item.labels) .. ")"
end

local function render_board_digest(M, issues, prs, closed_issues)
  local lines = {
    M._untrusted_issue_data_begin,
    "Open items snapshot:",
  }
  for _, item in ipairs(issues or {}) do
    if #lines >= 52 then
      break
    end
    table.insert(lines, "#" .. tostring(item.number)
      .. " [" .. state_label(M, item.labels) .. "] "
      .. first_chars(M, item.title, 60))
  end
  for _, item in ipairs(prs or {}) do
    if #lines >= 52 then
      break
    end
    table.insert(lines, "#" .. tostring(item.number)
      .. " [" .. state_label(M, item.labels) .. "] "
      .. first_chars(M, item.title, 60))
  end
  table.insert(lines, "")
  table.insert(lines, "Recent closed issues for recurrence judgment:")
  for _, item in ipairs(closed_issues or {}) do
    if #lines >= 84 then
      break
    end
    table.insert(lines, render_closed_issue_line(M, item))
  end
  if type(closed_issues) ~= "table" or #closed_issues == 0 then
    table.insert(lines, "(none fetched)")
  end
  table.insert(lines, M._untrusted_issue_data_end)
  return table.concat(lines, "\n")
end

function M.board_digest_block(repo, tick)
  if tick == nil or tostring(tick) == "" then
    return ""
  end
  local key = "github-devloop/board-digest/" .. M.safe_repo(repo) .. "/" .. M.safe_updated_at(tick)
  local cached = cache_get(key)
  if cached ~= nil and cached ~= "" then
    return cached
  end

  local ok_issue, issue_result = pcall(M.gh_exec, { cmd = board_digest_issue_list_cmd(M, repo), timeout = 30 })
  local ok_pr, pr_result = pcall(M.gh_exec, { cmd = board_digest_pr_list_cmd(M, repo), timeout = 30 })
  local ok_closed, closed_result = pcall(M.gh_exec, { cmd = recent_closed_issue_list_cmd(M, repo), timeout = 30 })
  if not ok_issue or not ok_pr
    or type(issue_result) ~= "table" or issue_result.exit_code ~= 0
    or type(pr_result) ~= "table" or pr_result.exit_code ~= 0 then
    return ""
  end

  local closed_issues = nil
  if ok_closed and type(closed_result) == "table" and closed_result.exit_code == 0 then
    local ok_parse, parsed = pcall(parse_board_list, closed_result.stdout)
    if ok_parse then
      closed_issues = parsed
    end
  end

  local block = render_board_digest(
    M,
    parse_board_list(issue_result.stdout),
    parse_board_list(pr_result.stdout),
    closed_issues
  )
  cache_set(key, block)
  return block
end

function M.append_board_digest_to_proposal(proposal, repo, tick)
  local block = M.board_digest_block(repo, tick)
  if block == "" then
    return proposal
  end
  local body = tostring(proposal.body or "")
  local prefix = "\n\n"
  local neutralized = M.neutralize_untrusted_prompt_text(block)
  local remaining = M._max_body_len - #body - #prefix
  if remaining <= 0 then
    M.log_line("warn", "payloads", proposal.proposal_id, "BOARD_DIGEST", {
      "outcome=drop",
      "reason=body-budget-exhausted",
      "repo=" .. tostring(repo or ""),
      "tick=" .. tostring(tick or ""),
    })
    return proposal
  end
  if #neutralized > remaining then
    M.log_line("warn", "payloads", proposal.proposal_id, "BOARD_DIGEST", {
      "outcome=truncate",
      "reason=body-budget",
      "repo=" .. tostring(repo or ""),
      "tick=" .. tostring(tick or ""),
      "available=" .. tostring(remaining),
      "needed=" .. tostring(#neutralized),
    })
    neutralized = M.truncate_utf8(neutralized, remaining)
  end
  proposal.body = body .. prefix .. neutralized
  if #proposal.body > M._max_body_len then
    error("github-devloop: proposal board digest exceeds bounded body")
  end
  return proposal
end

function M.build_devloop_ready_payload(source)
  local payload = {
    schema = "github-devloop.ready.v1",
    proposal_id = source.proposal_id,
    dedup_key = M._dedup_key({
      "ready",
      tostring(source.dedup_key),
    }),
    source_ref = M.normalize_source_ref(source.source_ref),
  }
  local framing = bounded_framing(M, source.framing)
  if framing ~= nil then
    payload.framing = framing
  end
  return payload
end

function M.build_devloop_reviewing_payload(origin, pr_number, source_ref, version)
  local review_version = version or origin.impl_version
  return {
    schema = "github-devloop.reviewing.v1",
    proposal_id = origin.proposal_id,
    pr_number = pr_number,
    version = review_version,
    dedup_key = M._dedup_key({
      "reviewing",
      tostring(origin.proposal_id),
      tostring(review_version),
      tostring(pr_number),
    }),
    source_ref = M.normalize_source_ref(source_ref),
  }
end

function M.build_devloop_fixing_payload(origin, pr_number, review_fact, source_ref)
  local version = origin.impl_version
  if review_fact.fix_version ~= nil then
    version = review_fact.fix_version
  end
  local payload = {
    schema = "github-devloop.fixing.v1",
    proposal_id = origin.proposal_id,
    pr_number = pr_number,
    version = version,
    review_proposal_id = review_fact.review_proposal_id,
    review_dedup_key = review_fact.review_dedup_key,
    reviewed_head_sha = review_fact.reviewed_head_sha,
    dedup_key = M._dedup_key({
      "fixing",
      tostring(origin.proposal_id),
      tostring(version),
      tostring(pr_number),
      tostring(review_fact.review_dedup_key),
    }),
    source_ref = M.normalize_source_ref(source_ref),
  }
  local framing = bounded_framing(M, review_fact.framing or origin.framing)
  if framing ~= nil then
    payload.framing = framing
  end
  local blocking_gap = bounded_control_text(M, review_fact.blocking_gap, M._max_blocking_gap_len)
  if blocking_gap ~= nil then
    payload.blocking_gap = blocking_gap
  end
  if review_fact.gate_baseline_sha ~= nil then
    if not M._is_git_sha(review_fact.gate_baseline_sha) then
      error("github-devloop: invalid gate baseline sha")
    end
    payload.gate_baseline_sha = tostring(review_fact.gate_baseline_sha)
  end
  local gate_failure_excerpt = bounded_control_text(M, review_fact.gate_failure_excerpt, M._max_rollup_failure_summary_len)
  if gate_failure_excerpt ~= nil then
    payload.gate_failure_excerpt = gate_failure_excerpt
  end
  return payload
end

function M.build_devloop_review_meta_payload(unresolved, issue_proposal_id, issue_version, pr_number, n, source_ref)
  return {
    schema = "github-devloop.review-meta.v1",
    proposal_id = issue_proposal_id,
    review_proposal_id = unresolved.proposal_id,
    review_dedup_key = unresolved.dedup_key,
    version = issue_version,
    pr_number = pr_number,
    n = n,
    dedup_key = M._dedup_key({
      "review-meta",
      tostring(issue_proposal_id),
      tostring(issue_version),
      tostring(pr_number),
      tostring(n),
      tostring(unresolved.dedup_key),
    }),
    source_ref = M.normalize_source_ref(source_ref or unresolved.source_ref),
  }
end

function M.build_devloop_merge_ready_payload(issue_proposal_id, pr_number, version, review_fact, source_ref)
  return {
    schema = "github-devloop.merge-ready.v1",
    proposal_id = issue_proposal_id,
    pr_number = pr_number,
    version = version,
    review_proposal_id = review_fact and review_fact.review_proposal_id,
    review_dedup_key = review_fact and review_fact.review_dedup_key,
    reviewed_head_sha = review_fact and review_fact.reviewed_head_sha,
    dedup_key = M._dedup_key({
      "merge-ready",
      tostring(issue_proposal_id),
      tostring(version),
      tostring(pr_number),
      tostring(review_fact and review_fact.review_dedup_key or "review"),
    }),
    source_ref = M.normalize_source_ref(source_ref),
  }
end

function M.build_devloop_intake_candidate_payload(repo, issue_number, updated_at)
  local proposal_id = M.proposal_id(repo, issue_number)
  local source_ref = {
    kind = "external",
    ref = tostring(repo) .. "#issue/" .. tostring(issue_number),
  }
  return {
    schema = "github-devloop.intake-candidate.v1",
    repo = repo,
    issue_number = issue_number,
    proposal_id = proposal_id,
    dedup_key = M.intake_dedup_key(proposal_id, updated_at),
    source_ref = source_ref,
  }
end

function M.build_proposal(issue)
  local proposal_id = M.proposal_id(issue.repo, issue.number)
  local title = tostring(issue.title or "")
  if #title > M._max_title_len then
    title = M.truncate_utf8(title, M._max_title_len)
  end
  local body = "Judge the current GitHub issue from the full source content."
    .. "\nIssue: " .. tostring(issue.repo) .. "#" .. tostring(issue.number)
    .. "\nRecurrence: read recent closed issues in context; if this is the third same-class instance, reframe to a class solution or give an explicit waiver."

  return {
    schema = "consensus.proposal.v1",
    verdict_mode = "converge",
    proposal_id = proposal_id,
    title = title,
    body = body,
    content_fetch = issue.content_fetch,
    dedup_key = M.proposal_dedup_key(proposal_id, issue.updated_at),
    source_ref = M.normalize_source_ref(issue.source_ref),
  }
end

function M.build_board_proposal(issue, tick)
  return M.append_board_digest_to_proposal(M.build_proposal(issue), issue.repo, tick)
end

-- Thread the meta-judge's narrowing onto a re-raised next-round proposal so the next
-- angles converge instead of blindly re-judging the same question. The next round sees
-- ONLY the bounded convergence_question + prior-round digests (verdict + short reply),
-- never prior peer full text, preserving angle peer-invisibility. The `/loop/N` dedup
-- shape stays unchanged so the existing round parsing + budget endpoint still work.
local function apply_converge_fields(proposal, n, converge)
  proposal.round = n
  if type(converge) ~= "table" then
    return proposal
  end
  if converge.narrowed_question ~= nil and converge.narrowed_question ~= "" then
    proposal.convergence_question = converge.narrowed_question
  end
  if type(converge.angle_digests) == "table" then
    proposal.prior_round_digests = converge.angle_digests
  end
  return proposal
end

function M.build_loop_proposal(repo, issue_number, current, source_ref, n, converge, content_fetch)
  local issue = {
    repo = repo,
    number = issue_number,
    title = current.title,
    updated_at = current.updated_at,
    source_ref = source_ref,
    content_fetch = content_fetch,
  }
  local proposal = M.build_proposal(issue)
  proposal.dedup_key = proposal.dedup_key .. "/loop/" .. tostring(n)
  return apply_converge_fields(proposal, n, converge)
end

function M.build_board_loop_proposal(repo, issue_number, current, source_ref, n, converge, tick, content_fetch)
  return M.append_board_digest_to_proposal(M.build_loop_proposal(repo, issue_number, current, source_ref, n, converge, content_fetch), repo, tick)
end

function M.build_pr_review_proposal(repo, issue_number, pr_number, version, head_sha, current_issue, source_ref, pr_comments, content_fetch)
  local review_id = M.pr_review_proposal_id(repo, pr_number, version, head_sha)
  local title = "Review PR #" .. tostring(pr_number)
  if issue_number ~= nil then
    title = title .. " for issue #" .. tostring(issue_number)
  end
  if type(current_issue) == "table" and tostring(current_issue.title or "") ~= "" then
    title = "Review PR #" .. tostring(pr_number) .. ": " .. tostring(current_issue.title)
  end
  if #title > M._max_title_len then
    title = M.truncate_utf8(title, M._max_title_len)
  end

  local issue_title = type(current_issue) == "table" and tostring(current_issue.title or "") or ""
  if #issue_title > M._max_title_len then
    issue_title = M.truncate_utf8(issue_title, M._max_title_len)
  end
  issue_title = M.neutralize_untrusted_prompt_text(M._neutralize_fkst_markers(issue_title))
  local body = "Review the PR diff and decide whether it should advance to merge-ready."
    .. "\nEntity proposal: " .. tostring(issue_number ~= nil and M.proposal_id(repo, issue_number) or M.pr_proposal_id(repo, pr_number))
    .. "\nReviewed PR head: " .. tostring(head_sha)
    .. "\nIssue title: " .. issue_title
    .. "\nRead the local context bundle before judging."
  local issue_proposal_id = tostring(issue_number ~= nil and M.proposal_id(repo, issue_number) or M.pr_proposal_id(repo, pr_number))
  local ledger = M.review_prior_round_ledger(pr_comments, issue_proposal_id, version)
  if ledger ~= nil and ledger ~= "" then
    body = body
      .. "\nPrior review ledger:\n"
      .. ledger
      .. "\nJudge whether THE NAMED GAP is closed; new objections only for regressions introduced by the fix."
  end
  if #body > M._max_body_len then
    error("github-devloop: PR review proposal exceeds bounded body")
  end

  return {
    schema = "consensus.proposal.v1",
    verdict_mode = "gate",
    proposal_id = review_id,
    title = M.neutralize_untrusted_prompt_text(title),
    body = body,
    content_fetch = content_fetch,
    dedup_key = M._dedup_key({
      review_id,
      "review",
    }),
    source_ref = M.normalize_source_ref(source_ref),
  }
end

function M.build_board_pr_review_proposal(repo, issue_number, pr_number, version, head_sha, current_issue, source_ref, tick, pr_comments, content_fetch)
  return M.append_board_digest_to_proposal(M.build_pr_review_proposal(repo, issue_number, pr_number, version, head_sha, current_issue, source_ref, pr_comments, content_fetch), repo, tick)
end

function M.build_pr_review_loop_proposal(repo, issue_number, pr_number, version, head_sha, current_issue, source_ref, n, converge, pr_comments, content_fetch)
  local proposal = M.build_pr_review_proposal(repo, issue_number, pr_number, version, head_sha, current_issue, source_ref, pr_comments, content_fetch)
  proposal.dedup_key = proposal.dedup_key .. "/loop/" .. tostring(n)
  return apply_converge_fields(proposal, n, converge)
end

function M.build_board_pr_review_loop_proposal(repo, issue_number, pr_number, version, head_sha, current_issue, source_ref, n, converge, tick, pr_comments, content_fetch)
  return M.append_board_digest_to_proposal(M.build_pr_review_loop_proposal(repo, issue_number, pr_number, version, head_sha, current_issue, source_ref, n, converge, pr_comments, content_fetch), repo, tick)
end
end

return S
