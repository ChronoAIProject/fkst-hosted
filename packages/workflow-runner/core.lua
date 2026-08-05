-- Pure core of the scheduled-workflow runner.
--
-- Every decision lives here so the departments stay thin saga wrappers: reading
-- the dispatch facts off a run issue, validating a workflow definition,
-- substituting arguments as DATA, and rendering the run record the control plane
-- reads back.
--
-- Two cross-repository contracts pass through this file:
--
--  * `fkst-cron-dispatch:v1` — written by the control plane onto the run issue it
--    creates (`backend/src/reconcile/schedule_run_issue.rs`). Read-only here.
--  * `fkst-cron-run:v1` — the run record. The control plane's
--    `backend/src/schedule/marker.rs` parses this exact string, and its
--    `the_rendered_marker_matches_the_pinned_wire_format` test pins the literal.
--    A drift here silently breaks completion detection: a finished run would look
--    in-flight until its watchdog released it.

local M = {}

local toml = require("core.toml")

-- Matched with a Lua pattern, so `-` and `:` are escaped.
local DISPATCH_MARKER = "fkst%-cron%-dispatch:v1"

-- Output tails land in an issue comment: an unbounded one would push the human
-- part of the record out of view, and a step that prints a megabyte is exactly
-- the step whose tail you want to read.
local MAX_TAIL_BYTES = 4096

local STEP_KINDS = { run = true, task = true }

local function attribute(line, name)
  return line:match(name .. '="([^"]*)"')
end

--- The dispatch facts a run issue's body carries.
---
--- Returns `nil, nil` when the body is not a scheduled run at all — an ordinary
--- work issue must be a clean no-op. Returns `nil, err` when the marker is there
--- but unreadable, which is a hard failure: silently ignoring it would strand the
--- schedule until its watchdog fired, with nothing anywhere saying why.
function M.parse_dispatch(body)
  if type(body) ~= "string" then
    return nil, nil
  end
  for line in body:gmatch("[^\n]+") do
    if line:find(DISPATCH_MARKER) then
      local schedule = tonumber(attribute(line, "schedule"))
      local workflow = attribute(line, "workflow")
      local slot = attribute(line, "slot")
      if schedule == nil or workflow == nil or workflow == "" or slot == nil or slot == "" then
        return nil, "malformed fkst-cron-dispatch:v1 marker"
      end
      return {
        schedule_issue = schedule,
        workflow_id = workflow,
        slot = slot,
        manual = attribute(line, "manual") == "true",
        arguments = M.parse_arguments(body),
      }, nil
    end
  end
  return nil, nil
end

--- The fenced `toml` argument block a run issue carries.
---
--- A narrow reader rather than the full decoder: the control plane emits exactly
--- `key = "value"` lines with basic-string escaping, and accepting more here
--- would accept shapes the writer never produces.
function M.parse_arguments(body)
  local out = {}
  if type(body) ~= "string" then
    return out
  end
  local block = body:match("```toml\n(.-)```")
  if block == nil then
    return out
  end
  local decoded = toml.decode(block)
  if decoded == nil then
    return out
  end
  for key, value in pairs(decoded) do
    if key ~= "step" and type(value) == "string" then
      out[key] = value
    end
  end
  return out
end

--- Validate a decoded definition into an ordered step list.
---
--- Fail-closed on every shape problem, naming the offending step: a definition
--- that half-runs is worse than one that refuses, because half a pipeline can
--- publish half a result over a good one.
function M.validate_definition(definition)
  if type(definition) ~= "table" or type(definition.step) ~= "table" then
    return nil, "workflow definition must declare at least one [[step]]"
  end
  local steps, seen = {}, {}
  for index, step in ipairs(definition.step) do
    local id = step.id
    if type(id) ~= "string" or id == "" or id:find("[^%w%._%-]") then
      return nil, ("step %d has an invalid id"):format(index)
    end
    if seen[id] then
      -- Step ids key the run record's per-step outcomes, so a duplicate would
      -- make two steps indistinguishable in the history and the dashboard.
      return nil, ("duplicate step id %q"):format(id)
    end
    seen[id] = true
    if not STEP_KINDS[step.kind] then
      return nil,
        ("step %s has an unsupported kind %q (expected run or task)"):format(id, tostring(step.kind))
    end
    if step.kind == "run" and type(step.command) ~= "table" then
      return nil, ("run step %s must declare a `command` array"):format(id)
    end
    if step.kind == "task" and (type(step.prompt) ~= "string" or step.prompt == "") then
      return nil, ("task step %s must declare a prompt"):format(id)
    end
    steps[#steps + 1] = {
      index = index,
      id = id,
      kind = step.kind,
      command = step.command,
      prompt = step.prompt,
      timeout_secs = tonumber(step.timeout_secs) or 900,
    }
  end
  if #steps == 0 then
    return nil, "workflow definition declares no steps"
  end
  return steps, nil
end

--- Substitute `{{ name }}` placeholders with argument VALUES.
---
--- Substitution targets one argv ELEMENT or a prompt — never a shell string. The
--- caller hands argv straight to the runner, so a value containing `;` or a quote
--- is an ordinary argument rather than syntax.
---
--- An unknown placeholder is an ERROR, not an empty string: running a scrape with
--- a blank search term would produce a plausible, wrong result rather than a
--- visible failure.
function M.substitute(text, arguments)
  if type(text) ~= "string" then
    return nil, "substitution target must be a string"
  end
  local missing = nil
  local out = text:gsub("{{%s*([%w_]+)%s*}}", function(name)
    local value = arguments and arguments[name]
    if value == nil then
      missing = missing or name
      return ""
    end
    return value
  end)
  if missing ~= nil then
    return nil, ("workflow argument %q is referenced but not supplied"):format(missing)
  end
  return out, nil
end

--- Apply substitution across one step's argv or prompt.
function M.resolve_step(step, arguments)
  local resolved = {
    index = step.index,
    id = step.id,
    kind = step.kind,
    timeout_secs = step.timeout_secs,
  }
  if step.kind == "run" then
    local argv = {}
    for position, element in ipairs(step.command) do
      local value, err = M.substitute(tostring(element), arguments)
      if err ~= nil then
        return nil, err
      end
      argv[position] = value
    end
    if #argv == 0 then
      return nil, ("run step %s has an empty command"):format(step.id)
    end
    resolved.argv = argv
  else
    local prompt, err = M.substitute(step.prompt, arguments)
    if err ~= nil then
      return nil, err
    end
    resolved.prompt = prompt
  end
  return resolved, nil
end

--- Tail-truncate captured output to the comment budget.
function M.truncate_tail(text, max_bytes)
  local limit = max_bytes or MAX_TAIL_BYTES
  if type(text) ~= "string" then
    return ""
  end
  if #text <= limit then
    return text
  end
  return ("…(truncated, %d bytes omitted)\n%s"):format(#text - limit, text:sub(#text - limit + 1))
end

--- Strip everything that would terminate the enclosing HTML comment or its
--- attribute, and bound the length. A detail is free text from a failing step:
--- hostile to the format by default, not trusted to behave.
function M.sanitize_detail(detail)
  if type(detail) ~= "string" then
    return nil
  end
  local out = detail:gsub('[<>"]', "'"):gsub("%c", " ")
  out = out:gsub("^%s+", ""):gsub("%s+$", "")
  if #out > 200 then
    out = out:sub(1, 200)
  end
  if out == "" then
    return nil
  end
  return out
end

--- Encode per-step outcomes as the marker's `steps` attribute.
---
--- A separator-delimited scalar rather than embedded JSON: the value sits inside
--- a double-quoted HTML-comment attribute, and step ids are restricted to a
--- path-safe token set, so neither separator can occur in the data.
function M.render_steps(steps)
  local parts = {}
  for _, step in ipairs(steps or {}) do
    parts[#parts + 1] = ("%d:%s:%s:%s"):format(
      step.index,
      step.id,
      step.status,
      step.duration_s and tostring(math.floor(step.duration_s)) or ""
    )
  end
  return table.concat(parts, ";")
end

--- Render the `fkst-cron-run:v1` record.
---
--- Absent optional attributes are OMITTED rather than emitted empty, so a reader
--- never has to tell "absent" from "present but blank".
function M.render_run_marker(record)
  local fields = {
    ('slot="%s"'):format(record.slot),
    ('manual="%s"'):format(record.manual and "true" or "false"),
    ('status="%s"'):format(record.status),
    ('started="%s"'):format(record.started),
  }
  if record.ended ~= nil then
    fields[#fields + 1] = ('ended="%s"'):format(record.ended)
  end
  if record.issue ~= nil then
    fields[#fields + 1] = ('issue="%s"'):format(tostring(record.issue))
  end
  local detail = M.sanitize_detail(record.detail)
  if detail ~= nil then
    fields[#fields + 1] = ('detail="%s"'):format(detail)
  end
  local steps = M.render_steps(record.steps)
  if steps ~= "" then
    fields[#fields + 1] = ('steps="%s"'):format(steps)
  end
  return ("<!-- fkst-cron-run:v1 %s -->"):format(table.concat(fields, " "))
end

--- The comment body posted onto the DEFINITION issue: a human line, an optional
--- output tail, and the machine record.
function M.render_report(record)
  local body
  if record.status == "ok" then
    body = { ("✅ Scheduled run succeeded — slot `%s`."):format(record.slot), "" }
  else
    body = {
      ("❌ Scheduled run failed — slot `%s`.%s"):format(
        record.slot,
        record.detail and (" " .. record.detail) or ""
      ),
      "",
    }
  end
  if type(record.tail) == "string" and record.tail ~= "" then
    body[#body + 1] = "```"
    body[#body + 1] = record.tail
    body[#body + 1] = "```"
    body[#body + 1] = ""
  end
  body[#body + 1] = M.render_run_marker(record)
  return table.concat(body, "\n")
end

--- The definition path a workflow id resolves to.
---
--- Re-validated here even though the control plane validated the id, because THIS
--- is where it becomes a filesystem path, and a path built from an unvalidated
--- token is the classic traversal.
function M.definition_path(workflow_id)
  if type(workflow_id) ~= "string" or workflow_id == "" then
    return nil, "missing workflow id"
  end
  -- `plain = true` means the needle is LITERAL, so it must be `..` and not the
  -- escaped pattern `%.%.` — passing the escaped form here searches for those
  -- four characters and the traversal guard silently never fires.
  if workflow_id:find("[^%w%._%-]") or workflow_id:find("..", 1, true) then
    return nil, ("unsafe workflow id %q"):format(workflow_id)
  end
  return (".fkst/workflows/%s.toml"):format(workflow_id), nil
end

--- Pick the run issue this boot should service.
---
--- The LOWEST-numbered match wins, so two run issues open at once are worked in
--- the order they were created rather than in whatever order the listing came
--- back. Only issues assigned to this session's creator are eligible, mirroring
--- the control plane's own sole-assignee routing rule.
function M.select_run_issue(issues, creator_login)
  local chosen, chosen_dispatch = nil, nil
  for _, issue in ipairs(issues or {}) do
    local assignees = issue.assignees or {}
    local routed = #assignees == 1
      and creator_login ~= nil
      and tostring(assignees[1]):lower() == tostring(creator_login):lower()
    if routed then
      local dispatch = M.parse_dispatch(issue.body)
      if dispatch ~= nil and (chosen == nil or issue.number < chosen.number) then
        chosen, chosen_dispatch = issue, dispatch
      end
    end
  end
  return chosen, chosen_dispatch
end

M.MAX_TAIL_BYTES = MAX_TAIL_BYTES

return M
