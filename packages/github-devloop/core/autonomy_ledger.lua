local S = {}

function S.install(M)
local task_classes = {
  L0 = true,
  L1 = true,
  L2 = true,
  L3 = true,
  L4 = true,
  unknown = true,
}

local gate_states = {
  pass = true,
  fail = true,
  pending = true,
}

local audit_states = {
  ["true"] = true,
  ["false"] = true,
  pending = true,
  invalid_self_attested = true,
}

local required_gate_names = {
  "human_touch",
  "pre_merge_ci",
  "evidence_manifest",
  "post_merge_probe",
  "no_revert_reopen",
  "cost_budget",
}

local function normalize_gate_state(value)
  local state = tostring(value or "")
  if gate_states[state] then
    return state
  end
  return "pending"
end

local function normalize_task_class(value)
  local class = tostring(value or "")
  if task_classes[class] then
    return class
  end
  return "unknown"
end

local function label_name(label)
  if type(label) == "table" then
    if label.name ~= nil then
      return tostring(label.name)
    end
    if label.label ~= nil then
      return tostring(label.label)
    end
  elseif label ~= nil then
    return tostring(label)
  end
  return ""
end

local function task_class_from_label(label)
  local text = label_name(label)
  local found = text:match("[Aa][Vv][Mm][%-%_: ]*([Ll][0-4])")
    or text:match("[Tt][Aa][Ss][Kk][%-%_ ]*[Cc][Ll][Aa][Ss][Ss][%-%_: ]*([Ll][0-4])")
    or text:match("[Cc][Oo][Mm][Pp][Ee][Tt][Ee][Nn][Cc][Ee][%-%_: ]*([Ll][0-4])")
  if found == nil then
    return nil
  end
  return found:upper()
end

local title_patterns = {
  { class = "L4", patterns = { "security", "auth", "credential", "secret", "cross%-repo", "api" } },
  { class = "L3", patterns = { "engine", "scheduler", "recovery", "conformance", "liveness", "saga", "watchdog" } },
  { class = "L2", patterns = { "refactor", "cross%-module", "architecture", "adapter", "ports" } },
  { class = "L1", patterns = { "fix", "bug", "test", "harness", "regression" } },
  { class = "L0", patterns = { "docs", "documentation", "readme", "comment", "chore" } },
}

function M.autonomy_task_class(issue)
  if type(issue) == "table" then
    for _, label in ipairs(issue.labels or {}) do
      local class = task_class_from_label(label)
      if class ~= nil then
        return normalize_task_class(class)
      end
    end
    local title = tostring(issue.title or ""):lower()
    for _, entry in ipairs(title_patterns) do
      for _, pattern in ipairs(entry.patterns) do
        if title:find(pattern) ~= nil then
          return entry.class
        end
      end
    end
  end
  return "unknown"
end

function M.autonomy_valid_autonomous_merge(gates)
  local has_pending = false
  for _, name in ipairs(required_gate_names) do
    local state = normalize_gate_state(type(gates) == "table" and gates[name] or nil)
    if state == "fail" then
      return "false"
    end
    if state == "pending" then
      has_pending = true
    end
  end
  if has_pending then
    return "pending"
  end
  return "true"
end

function M.autonomy_merge_rounds(version)
  return M.version_loop_round(version) + M.version_fix_round(version)
end

function M.autonomy_post_merge_probe_gate(pr, opts)
  local green, reason = M.evaluate_ci_status_gate(pr, opts)
  if green then
    return "pass", reason
  end
  return "fail", reason
end

function M.autonomy_result_record(repo, issue_number, merge_ready, issue, post_merge_pr)
  local human_touch_count = 0
  local post_merge_probe = "pending"
  if post_merge_pr ~= nil then
    post_merge_probe = M.autonomy_post_merge_probe_gate(post_merge_pr, {
      repo = repo,
      dept = "merge",
      proposal_id = tostring(merge_ready.proposal_id),
    })
  end
  local gates = {
    human_touch = human_touch_count == 0 and "pass" or "fail",
    pre_merge_ci = "pass",
    evidence_manifest = "pending",
    post_merge_probe = post_merge_probe,
    no_revert_reopen = "pending",
    cost_budget = "pending",
  }
  return {
    schema = "github-devloop.autonomy-result.v1",
    proposal_id = tostring(merge_ready.proposal_id),
    repo = tostring(repo or ""),
    issue_number = issue_number ~= nil and tostring(issue_number) or "",
    pr_number = tostring(merge_ready.pr_number),
    version = tostring(merge_ready.version),
    head_sha = tostring(merge_ready.reviewed_head_sha),
    task_class = M.autonomy_task_class(issue),
    human_touch_count = human_touch_count,
    pre_merge_ci = gates.pre_merge_ci,
    rounds = M.autonomy_merge_rounds(merge_ready.version),
    retry_count = M.version_fix_round(merge_ready.version),
    codex_calls = nil,
    gates = gates,
    valid_autonomous_merge = M.autonomy_valid_autonomous_merge(gates),
  }
end

local function autonomy_result_parts(record)
  if type(record) ~= "table" then
    error("github-devloop: invalid autonomy result record")
  end
  local proposal_id = tostring(record.proposal_id or "")
  local repo = tostring(record.repo or "")
  local issue_number = tostring(record.issue_number or "")
  local pr_number = tostring(record.pr_number or "")
  local version = tostring(record.version or "")
  local head_sha = tostring(record.head_sha or "")
  local task_class = normalize_task_class(record.task_class)
  local human_touch_count = tonumber(record.human_touch_count)
  local rounds = tonumber(record.rounds)
  local retry_count = tonumber(record.retry_count)
  local codex_calls = record.codex_calls
  local gates = type(record.gates) == "table" and record.gates or {}
  local valid = M.autonomy_valid_autonomous_merge(gates)
  if valid ~= "true" and valid ~= "false" and valid ~= "pending" then
    error("github-devloop: invalid autonomy result predicate")
  end
  if not M._is_path_safe_key(proposal_id, M._max_key_len)
    or not M._is_path_safe_key(repo, M._max_key_len)
    or not M._is_positive_pr_number(issue_number)
    or not M._is_positive_pr_number(pr_number)
    or not M._is_bounded_string(version, M._max_dedup_len)
    or not M._is_git_sha(head_sha)
    or human_touch_count == nil or human_touch_count < 0 or human_touch_count % 1 ~= 0
    or rounds == nil or rounds < 0 or rounds % 1 ~= 0
    or retry_count == nil or retry_count < 0 or retry_count % 1 ~= 0 then
    error("github-devloop: invalid autonomy result marker")
  end
  local codex_calls_value = "null"
  if codex_calls ~= nil then
    local parsed = tonumber(codex_calls)
    if parsed == nil or parsed < 0 or parsed % 1 ~= 0 then
      error("github-devloop: invalid autonomy result codex calls")
    end
    codex_calls_value = tostring(parsed)
  end
  return {
    proposal_id = proposal_id,
    repo = repo,
    issue_number = issue_number,
    pr_number = pr_number,
    version = version,
    head_sha = head_sha,
    task_class = task_class,
    human_touch_count = human_touch_count,
    rounds = rounds,
    retry_count = retry_count,
    codex_calls_value = codex_calls_value,
    gates = gates,
    valid = valid,
  }
end

function M.autonomy_result_marker_attrs(record)
  local parts = autonomy_result_parts(record)
  return ' repo="' .. parts.repo
    .. '" issue="' .. parts.issue_number
    .. '" task_class="' .. parts.task_class
    .. '" human_touch_count="' .. tostring(parts.human_touch_count)
    .. '" pre_merge_ci="' .. normalize_gate_state(parts.gates.pre_merge_ci)
    .. '" rounds="' .. tostring(parts.rounds)
    .. '" retry_count="' .. tostring(parts.retry_count)
    .. '" codex_calls="' .. parts.codex_calls_value
    .. '" gate_human_touch="' .. normalize_gate_state(parts.gates.human_touch)
    .. '" gate_evidence_manifest="' .. normalize_gate_state(parts.gates.evidence_manifest)
    .. '" gate_post_merge_probe="' .. normalize_gate_state(parts.gates.post_merge_probe)
    .. '" post_merge_probe_green="' .. normalize_gate_state(parts.gates.post_merge_probe)
    .. '" gate_no_revert_reopen="' .. normalize_gate_state(parts.gates.no_revert_reopen)
    .. '" gate_cost_budget="' .. normalize_gate_state(parts.gates.cost_budget)
    .. '" valid_autonomous_merge="' .. parts.valid .. '"'
end

function M.autonomy_result_marker(record)
  local parts = autonomy_result_parts(record)
  return '<!-- fkst:github-devloop:autonomy-result:v1 proposal="' .. parts.proposal_id
    .. '" repo="' .. parts.repo
    .. '" issue="' .. parts.issue_number
    .. '" pr="' .. parts.pr_number
    .. '" version="' .. parts.version
    .. '" head_sha="' .. parts.head_sha
    .. '" task_class="' .. parts.task_class
    .. '" human_touch_count="' .. tostring(parts.human_touch_count)
    .. '" pre_merge_ci="' .. normalize_gate_state(parts.gates.pre_merge_ci)
    .. '" rounds="' .. tostring(parts.rounds)
    .. '" retry_count="' .. tostring(parts.retry_count)
    .. '" codex_calls="' .. parts.codex_calls_value
    .. '" gate_human_touch="' .. normalize_gate_state(parts.gates.human_touch)
    .. '" gate_evidence_manifest="' .. normalize_gate_state(parts.gates.evidence_manifest)
    .. '" gate_post_merge_probe="' .. normalize_gate_state(parts.gates.post_merge_probe)
    .. '" post_merge_probe_green="' .. normalize_gate_state(parts.gates.post_merge_probe)
    .. '" gate_no_revert_reopen="' .. normalize_gate_state(parts.gates.no_revert_reopen)
    .. '" gate_cost_budget="' .. normalize_gate_state(parts.gates.cost_budget)
    .. '" valid_autonomous_merge="' .. parts.valid .. '"'
    .. ' -->'
end

function M.autonomy_result_record_from_marker(marker, comment, proposal_id, pr_number, version, head_sha)
  local marker_proposal = marker:match('proposal="([^"]+)"')
  local marker_pr = marker:match('pr="([^"]+)"')
  local marker_version = marker:match('version="([^"]*)"')
  local marker_head_sha = marker:match('head_sha="([^"]+)"')
  local task_class = normalize_task_class(marker:match('task_class="([^"]+)"'))
  local valid = marker:match('valid_autonomous_merge="([^"]+)"')
  local human_touch_count = tonumber(marker:match('human_touch_count="(%d+)"'))
  local rounds = tonumber(marker:match('rounds="(%d+)"'))
  local retry_count = tonumber(marker:match('retry_count="(%d+)"'))
  local codex_calls_raw = marker:match('codex_calls="([^"]+)"')
  local gates = {
    human_touch = normalize_gate_state(marker:match('gate_human_touch="([^"]+)"')),
    pre_merge_ci = normalize_gate_state(marker:match('pre_merge_ci="([^"]+)"')),
    evidence_manifest = normalize_gate_state(marker:match('gate_evidence_manifest="([^"]+)"')),
    post_merge_probe = normalize_gate_state(
      marker:match('post_merge_probe_green="([^"]+)"') or marker:match('gate_post_merge_probe="([^"]+)"')
    ),
    no_revert_reopen = normalize_gate_state(marker:match('gate_no_revert_reopen="([^"]+)"')),
    cost_budget = normalize_gate_state(marker:match('gate_cost_budget="([^"]+)"')),
  }
  if marker_proposal == tostring(proposal_id)
    and tostring(marker_pr) == tostring(pr_number)
    and tostring(marker_version) == tostring(version)
    and tostring(marker_head_sha) == tostring(head_sha)
    and M._is_git_sha(marker_head_sha)
    and human_touch_count ~= nil
    and rounds ~= nil
    and retry_count ~= nil
    and (valid == "true" or valid == "false" or valid == "pending") then
    local codex_calls = nil
    if codex_calls_raw ~= "null" then
      codex_calls = tonumber(codex_calls_raw)
      if codex_calls == nil or codex_calls < 0 or codex_calls % 1 ~= 0 then
        return nil
      end
    end
    return {
      proposal_id = marker_proposal,
      repo = marker:match('repo="([^"]+)"'),
      issue_number = tonumber(marker:match('issue="(%d+)"')),
      pr_number = tonumber(marker_pr),
      version = marker_version,
      head_sha = marker_head_sha,
      task_class = task_class,
      human_touch_count = human_touch_count,
      pre_merge_ci = normalize_gate_state(marker:match('pre_merge_ci="([^"]+)"')),
      rounds = rounds,
      retry_count = retry_count,
      codex_calls = codex_calls,
      gates = gates,
      valid_autonomous_merge = M.autonomy_valid_autonomous_merge(gates),
      comment_created_at = M._comment_created_at(comment),
    }
  end
  return nil
end

function M.autonomy_result_fact(comments, proposal_id, pr_number, version, head_sha)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:autonomy%-result:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local fact = M.autonomy_result_record_from_marker(marker, comment, proposal_id, pr_number, version, head_sha)
      if fact ~= nil then
        return fact
      end
    end
  end
  return nil
end

function M.autonomy_audit_valid_autonomous_merge(fact, opts)
  if type(fact) ~= "table" then
    return nil
  end
  local repo = tostring((type(opts) == "table" and opts.repo) or fact.repo or "")
  local head_sha = tostring((type(opts) == "table" and opts.merge_commit_sha) or fact.merge_commit_sha or fact.head_sha or "")
  if repo == "" or not M._is_git_sha(head_sha) then
    return {
      valid_autonomous_merge = "invalid_self_attested",
      reason = "missing-audit-source",
    }
  end
  local pr = {
    head_sha = head_sha,
    status_check_rollup = type(opts) == "table" and opts.status_check_rollup or {},
  }
  local green, reason = M.evaluate_ci_status_gate(pr, {
    repo = repo,
    dept = "autonomy-auditor",
    proposal_id = tostring(fact.proposal_id or ""),
  })
  local claimed_probe = normalize_gate_state(type(fact.gates) == "table" and fact.gates.post_merge_probe or nil)
  if green then
    return {
      valid_autonomous_merge = M.autonomy_valid_autonomous_merge({
        human_touch = type(fact.gates) == "table" and fact.gates.human_touch or nil,
        pre_merge_ci = type(fact.gates) == "table" and fact.gates.pre_merge_ci or nil,
        evidence_manifest = type(fact.gates) == "table" and fact.gates.evidence_manifest or nil,
        post_merge_probe = "pass",
        no_revert_reopen = type(fact.gates) == "table" and fact.gates.no_revert_reopen or nil,
        cost_budget = type(fact.gates) == "table" and fact.gates.cost_budget or nil,
      }),
      reason = "audited",
      gates = {
        post_merge_probe = "pass",
      },
    }
  end
  if claimed_probe == "pass" then
    return {
      valid_autonomous_merge = "invalid_self_attested",
      reason = tostring(reason or "missing-post-merge-probe-run"),
      gates = {
        post_merge_probe = "fail",
      },
    }
  end
  local state = "pending"
  if tostring(reason or "") == "rollup-red" then
    state = "invalid_self_attested"
  end
  return {
    valid_autonomous_merge = state,
    reason = tostring(reason or "post-merge-probe-not-green"),
    gates = {
      post_merge_probe = "fail",
    },
  }
end

function M.autonomy_audited_result_fact(comments, proposal_id, pr_number, version, head_sha, opts)
  local fact = M.autonomy_result_fact(comments, proposal_id, pr_number, version, head_sha)
  if fact == nil then
    return nil
  end
  local audit = M.autonomy_audit_valid_autonomous_merge(fact, opts or {})
  if type(audit) == "table" and audit.valid_autonomous_merge ~= nil then
    local state = tostring(audit.valid_autonomous_merge)
    if not audit_states[state] then
      state = "invalid_self_attested"
    end
    fact.audited_valid_autonomous_merge = state
    fact.audit_reason = audit.reason
    fact.audit_gates = audit.gates
  end
  return fact
end

end

return S
