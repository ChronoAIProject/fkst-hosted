local core = require("core")
local ports_seam = require("forge.ports")
local saga = require("workflow.saga")

local M = {}

local spec = {
  consumes = { "external_pr_scan", "external_pr_candidate" },
  ephemeral = { "external_pr_candidate" },
  produces = { "external_pr_candidate" },
  fanout = { "external_pr_scan", "external_pr_candidate" },
  stall_window = "30s",
}

local pr_view_fields = "title,headRefName,baseRefName,state,createdAt,updatedAt,author,comments,assignees"
local bridge_issue_view_fields = "number,title,state,url,labels,comments,author"

local function bare_queue(queue)
  return tostring(queue or ""):match("%.([^%.]+)$") or tostring(queue or "")
end

local function read_pr(github, repo, pr_number)
  local result = github.pr_cli_view(repo, pr_number, pr_view_fields, 30)
  local decoded = core.decode_json_object(result and result.stdout or "{}", "PR view")
  if type(decoded) ~= "table" then
    error("github-external-pr-intake: invalid-pr-view: PR view is not an object")
  end
  decoded.number = decoded.number or pr_number
  return core.normalize_pr(decoded, repo)
end

local function search_open_bridge_issues(github, repo, pr_number, managed)
  local result = github.issue_search(repo, core.bridge_search_query(repo, pr_number), "number,title,state,author,body,url", 30)
  local issues = core.decode_json_list(result and result.stdout or "[]", "bridge issue search")
  local found = {}
  for _, issue in ipairs(issues) do
    if core.trusted_author(issue, managed)
      and tostring(issue.state or ""):upper() ~= "CLOSED"
      and tostring(issue.body or ""):find(core.bridge_search_query(repo, pr_number), 1, true) ~= nil then
      table.insert(found, {
        issue_number = tonumber(issue.number),
        source = "issue-search",
      })
    end
  end
  table.sort(found, function(left, right)
    return tonumber(left.issue_number or 0) < tonumber(right.issue_number or 0)
  end)
  return found
end

local function search_all_bridge_issues(github, repo, pr_number, managed)
  local result = github.issue_search(repo, core.bridge_search_query(repo, pr_number), "number,title,state,author,body,url", 30)
  local issues = core.decode_json_list(result and result.stdout or "[]", "bridge issue search")
  local found = {}
  for _, issue in ipairs(issues) do
    if core.trusted_author(issue, managed)
      and tostring(issue.body or ""):find(core.bridge_search_query(repo, pr_number), 1, true) ~= nil then
      local normalized = core.normalize_issue(issue)
      if normalized.number ~= nil then
        table.insert(found, normalized)
      end
    end
  end
  table.sort(found, function(left, right)
    return tonumber(left.number or 0) < tonumber(right.number or 0)
  end)
  return found
end

local function search_bridge_issues(github, repo, pr_number, managed)
  return search_open_bridge_issues(github, repo, pr_number, managed)[1]
end

local function read_bridge_issue(github, repo, issue_number)
  local result = github.issue_view(repo, issue_number, bridge_issue_view_fields, 30)
  local decoded = core.decode_json_object(result and result.stdout or "{}", "bridge issue view")
  decoded.number = decoded.number or issue_number
  return core.normalize_issue(decoded)
end

local function write_comment(github, repo, pr_number, pr, issue_number)
  local path = core.body_file_path(repo, pr_number, "marker")
  file.write(path, core.bridge_marker(repo, pr_number, issue_number) .. "\n")
  return github.pr_comment(repo, pr_number, path, 30)
end

local function write_handled_comment(github, repo, pr, issue, signal)
  local path = core.body_file_path(repo, pr.number, "handled")
  file.write(path, core.handled_comment_body(repo, pr, issue, signal) .. "\n")
  return github.pr_comment(repo, pr.number, path, 30)
end

local function create_bridge_issue(github, repo, pr)
  local path = core.body_file_path(repo, pr.number, "issue")
  file.write(path, core.bridge_issue_body(repo, pr))
  local result = github.issue_create(repo, core.bridge_issue_title(pr), path, {}, {}, 30)
  local issue_number = core.parse_created_issue_number(result and result.stdout)
  if issue_number == nil then
    error("github-external-pr-intake: missing-issue-number: bridge issue create did not return an issue number")
  end
  return issue_number
end

local function reconcile_created_bridge_issue(github, repo, pr_number, managed, created_issue_number)
  local found = search_open_bridge_issues(github, repo, pr_number, managed)
  local canonical = found[1] and found[1].issue_number or created_issue_number
  local seen = {}
  if canonical ~= nil then
    seen[tonumber(canonical)] = true
  end
  for _, issue in ipairs(found) do
    local issue_number = tonumber(issue.issue_number)
    if issue_number ~= nil and not seen[issue_number] then
      seen[issue_number] = true
      github.issue_close(repo, issue_number, 30)
    end
  end
  if created_issue_number ~= nil
    and canonical ~= nil
    and tonumber(created_issue_number) ~= tonumber(canonical)
    and not seen[tonumber(created_issue_number)] then
    github.issue_close(repo, created_issue_number, 30)
  end
  return canonical
end

local function self_only_claim(pr, self_login)
  if self_login == nil or self_login == "" then
    return false
  end
  local seen = false
  for _, assignee in ipairs(pr.assignees or {}) do
    local login = core.strip_bot_login_suffix(assignee)
    if login == self_login then
      seen = true
    elseif login ~= nil and login ~= "" then
      return false
    end
  end
  return seen
end

local function ensure_claim(github, repo, pr, self_login)
  if self_only_claim(pr, self_login) then
    return true, pr
  end
  if #(pr.assignees or {}) > 0 then
    return false, pr
  end
  github.issue_assign(repo, pr.number, self_login, 30)
  local fresh = read_pr(github, repo, pr.number)
  if self_only_claim(fresh, self_login) then
    return true, fresh
  end
  return false, fresh
end

local function existing_bridge(github, repo, pr, managed)
  return core.find_pr_bridge_marker(pr.comments, repo, pr.number, managed)
    or search_bridge_issues(github, repo, pr.number, managed)
end

local function maybe_record_missing_pr_marker(github, repo, pr, bridge, managed, self_login)
  if bridge == nil or bridge.issue_number == nil or bridge.source == "pr-marker" then
    return
  end
  if core.find_pr_bridge_marker(pr.comments, repo, pr.number, managed) ~= nil then
    return
  end
  if core.write_enabled() and self_only_claim(pr, self_login) then
    write_comment(github, repo, pr.number, pr, bridge.issue_number)
  end
end

local function bridge_issue_is_handled(issue, repo, managed)
  if issue == nil or issue.number == nil then
    return nil
  end
  return core.find_bridge_issue_merged_signal(issue, repo, issue.number, managed)
end

local function acknowledge_handled_bridge(github, repo, pr, issue, signal, managed)
  if pr == nil or issue == nil or issue.number == nil or signal == nil then
    return "skip-unhandled"
  end
  if core.find_pr_handled_marker(pr.comments, repo, pr.number, issue.number, managed) ~= nil then
    if tostring(pr.state or ""):upper() == "OPEN" and core.write_enabled() then
      github.pr_close(repo, pr.number, 30)
      return "closed-after-existing-ack"
    end
    return "deduped-handled"
  end
  if not core.write_enabled() then
    return "would-acknowledge-handled"
  end
  write_handled_comment(github, repo, pr, issue, signal)
  if tostring(pr.state or ""):upper() == "OPEN" then
    github.pr_close(repo, pr.number, 30)
  end
  return "acknowledged-handled"
end

local function maybe_acknowledge_existing_bridge(github, repo, pr, bridge, managed)
  if bridge == nil or bridge.issue_number == nil then
    return nil
  end
  local issue = read_bridge_issue(github, repo, bridge.issue_number)
  local signal = bridge_issue_is_handled(issue, repo, managed)
  if signal == nil then
    return nil
  end
  return acknowledge_handled_bridge(github, repo, pr, issue, signal, managed)
end

local function maybe_acknowledge_bridge_from_scan(github, repo, pr, managed)
  if not core.is_external_candidate(pr, managed, now()) then
    return nil
  end
  local fresh_pr = read_pr(github, repo, pr.number)
  if not core.is_external_candidate(fresh_pr, managed, now()) then
    return nil
  end
  for _, candidate in ipairs(search_all_bridge_issues(github, repo, fresh_pr.number, managed)) do
    local issue = read_bridge_issue(github, repo, candidate.number)
    local signal = bridge_issue_is_handled(issue, repo, managed)
    if signal ~= nil then
      return acknowledge_handled_bridge(github, repo, fresh_pr, issue, signal, managed)
    end
  end
  return nil
end

local function handle_candidate(github, payload)
  local source_repo, source_pr = core.parse_source_ref(payload and payload.source_ref)
  local repo = tostring(payload.repo or source_repo)
  local pr_number = core.safe_number(payload.number or source_pr, "candidate pr")
  if repo ~= source_repo or pr_number ~= source_pr then
    error("github-external-pr-intake: source-ref-mismatch: candidate payload does not match source_ref")
  end

  local action = "skipped"
  with_lock(core.bridge_lock_key(repo, pr_number), function()
    local managed = core.managed_bot_logins()
    local pr = read_pr(github, repo, pr_number)
    if not core.is_external_candidate(pr, managed, now()) then
      action = "skip-not-external"
      return
    end

    local bridge = existing_bridge(github, repo, pr, managed)
    if bridge ~= nil then
      action = maybe_acknowledge_existing_bridge(github, repo, pr, bridge, managed)
        or ("deduped-" .. tostring(bridge.source))
      return
    end

    if not core.write_enabled() then
      action = "would-create-bridge"
      return
    end

    local self_login = core.current_bot_login()
    local claimed
    claimed, pr = ensure_claim(github, repo, pr, self_login)
    if not claimed then
      action = "skip-claimed-by-other"
      return
    end

    if not core.is_external_candidate(pr, managed, now()) then
      action = "skip-not-external-after-claim"
      return
    end
    bridge = existing_bridge(github, repo, pr, managed)
    if bridge ~= nil then
      maybe_record_missing_pr_marker(github, repo, pr, bridge, managed, self_login)
      action = maybe_acknowledge_existing_bridge(github, repo, pr, bridge, managed)
        or ("deduped-after-claim-" .. tostring(bridge.source))
      return
    end

    if not self_only_claim(pr, self_login) then
      action = "skip-lost-claim"
      return
    end
    local issue_number = create_bridge_issue(github, repo, pr)
    local canonical_issue_number = reconcile_created_bridge_issue(github, repo, pr_number, managed, issue_number)
    pr = read_pr(github, repo, pr_number)
    if core.find_pr_bridge_marker(pr.comments, repo, pr_number, managed) ~= nil then
      action = "created-bridge-marker-already-present"
      return
    end
    if not self_only_claim(pr, self_login) then
      action = "created-bridge-marker-deferred-lost-claim"
      return
    end
    write_comment(github, repo, pr_number, pr, canonical_issue_number)
    action = "created-bridge"
  end)
  core.log_line("info", "external_pr_intake", core.dedup_key(repo, pr_number), "ACTION", {
    "action=" .. tostring(action),
  })
end

local function handle_scan(github, event)
  local repo = core.required_repo()
  local managed = core.managed_bot_logins()
  local result = github.pr_list(repo, 30)
  for _, raw in ipairs(core.parse_pr_list(result and result.stdout or "[]")) do
    local pr = core.normalize_pr(raw, repo)
    if core.is_external_candidate(pr, managed, now()) then
      local handled_action = nil
      with_lock(core.bridge_lock_key(repo, pr.number), function()
        handled_action = maybe_acknowledge_bridge_from_scan(github, repo, pr, managed)
      end)
      if handled_action ~= nil then
        core.log_line("info", "external_pr_intake", core.dedup_key(repo, pr.number), "ACTION", {
          "action=" .. tostring(handled_action),
        })
      else
        local payload = {
          schema = "github-external-pr-intake.v1",
          repo = repo,
          number = pr.number,
          updated_at = pr.updated_at,
          dedup_key = core.dedup_key(repo, pr.number),
          source_ref = core.source_ref(repo, pr.number),
        }
        core.log_line("info", "external_pr_intake", payload.dedup_key, "RAISE", {
          "queue=external_pr_candidate",
        })
        raise("external_pr_candidate", payload)
      end
    end
  end
end

local function make_department(ports)
  local function done(_event)
    return false
  end

  local function act(event)
    local queue = bare_queue(event and event.queue)
    core.log_entry("external_pr_intake", event, "external-pr-intake", "poll")
    if queue == "external_pr_scan" then
      return handle_scan(ports.github, event)
    end
    if queue == "external_pr_candidate" then
      if type(event and event.payload) ~= "table" then
        error("github-external-pr-intake: invalid-payload: external_pr_candidate payload must be a table")
      end
      return handle_candidate(ports.github, event.payload)
    end
    error("github-external-pr-intake: unsupported-queue: " .. tostring(event and event.queue))
  end

  local previous_pipeline = _G.pipeline
  local department = saga.department(spec, {
    done = done,
    act = act,
    wrap = core.wrap_pipeline_failure,
    name = "external_pr_intake",
  })
  department.pipeline = _G.pipeline
  _G.pipeline = previous_pipeline
  return department
end

M = ports_seam.install(make_department)
M.make_department = make_department
_G.pipeline = M.pipeline

return M
