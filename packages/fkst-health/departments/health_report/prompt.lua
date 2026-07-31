-- The judgment codex's context and prompt, plus the fallback body used when it does
-- not answer.
--
-- THE CODEX DOES NOT DECIDE THE STATUS. It receives the rules-derived verdict and
-- explains it in the session's own terms. The prompt says so explicitly, and the
-- department ignores anything in the reply that looks like a competing verdict -- the
-- emitted `status` is always the core's.
--
-- SECRET HYGIENE. Only the core's evidence -- counts, booleans, and engine-owned
-- queue names -- reaches the context file. No environment value, no log text, and no
-- credential path is ever written here. The control-plane collector redacts every
-- byte on the way out of the pod, but that is defence in depth, not licence.
local P = {}

-- Reasoning effort is NOT settable per spawn: the engine's codex options are
-- prompt/context/worktree/sandbox/timeout/identity only, and effort is session-wide
-- codex configuration the control plane owns. Cost is bounded here instead by a small
-- fixed context and a short timeout -- this runs every ten minutes for every session
-- on the platform, so the bound matters more than the knob.
P.timeout_seconds = 180

local context_value_ceiling = 256

local function line(out, key, value)
  local text = tostring(value == nil and "" or value):gsub("%s+", " ")
  table.insert(out, tostring(key) .. ": " .. text:sub(1, context_value_ceiling))
end

--- The evidence context file's contents: the rules-derived verdict followed by the
--- observations that produced it, one per line.
function P.context(verdict, session_id, fault)
  local out = { "# fkst-health evidence for the last window", "" }
  line(out, "session_id", session_id)
  line(out, "rules_derived_status", verdict.status)
  line(out, "rules_derived_headline", verdict.headline)
  line(out, "confidence", verdict.confidence)
  table.insert(out, "")
  table.insert(out, "## observations")
  for _, entry in ipairs(type(verdict.evidence) == "table" and verdict.evidence or {}) do
    line(out, entry.key, entry.value)
  end
  if type(fault) == "table" then
    table.insert(out, "")
    table.insert(out, "## the dominant failure")
    line(out, "work_item", fault.work_item)
    line(out, "department", fault.dept)
    line(out, "queue", fault.queue)
    line(out, "attempts_before_dead_letter", fault.attempts)
    line(out, "dead_letters_for_this_queue", fault.count)
    line(out, "permanent", fault.permanent)
    -- The terminal error text. Already reduced to one bounded line upstream; the
    -- ceiling above bounds it again before it reaches the judge.
    line(out, "terminal_error", fault.reason)
  end
  table.insert(out, "")
  table.insert(out, "## open work items")
  local items = type(verdict.work_items) == "table" and verdict.work_items or {}
  if #items == 0 then
    table.insert(out, "none observed")
  end
  for _, item in ipairs(items) do
    line(out, "issue #" .. tostring(item.number), tostring(item.state) .. " / " .. tostring(item.progress))
  end
  return table.concat(out, "\n") .. "\n"
end

--- The prompt handed to the read-only judge.
---
--- `context_path` is ABSOLUTE. The judge runs with the project checkout as its cwd --
--- a read-only-sandbox codex refuses to start outside a git repository -- and reads
--- its evidence from this path instead, which is the shape every judgment codex in
--- this catalog uses.
function P.build(verdict, context_path, fault)
  return table.concat({
    "You are the health reporter for one running fkst coding session.",
    "Read the evidence file at " .. tostring(context_path) .. " and nothing else.",
    "Do not edit files. Do not run gh. Do not run git. Do not inspect the repository.",
    "",
    "The status has ALREADY been decided by deterministic rules: "
      .. tostring(verdict.status)
      .. ". You are not deciding it and you may not contradict it.",
    "Write 3 to 6 short sentences of plain markdown explaining, in this session's own",
    "terms, what that status means right now and what a human should do about it.",
    "",
    (type(fault) == "table"
      and "The evidence names a specific failure. Your answer MUST say, concretely: "
        .. "WHERE it is failing (the work item and department), WHAT it costs the user "
        .. "if they do nothing, WHY it failed (quote the terminal error), and WHAT THE "
        .. "USER SHOULD DO. You are reporting only -- never claim you fixed or will fix "
        .. "anything."
      or "Say what a human should watch for next."),
    "",
    "Hard constraints:",
    "- Do not assert any progress, failure, or cause that is not in the evidence.",
    "- Do not restate a different status, and do not hedge the given one.",
    "- If the evidence is thin, say plainly that little could be observed.",
    "- Do not invent issue numbers, file names, commits, or error messages.",
    "- Output markdown prose only: no front matter, no headings, no code fences.",
  }, "\n")
end

-- WHAT A FAILING REPORT MUST ANSWER.
--
-- Four questions, in this order, or the report is worthless to the person reading it:
--   1. WHERE   -- which work item / department / queue
--   2. IMPACT  -- what it costs them if they do nothing
--   3. WHY     -- the actual terminal error, not a count
--   4. WHAT TO DO -- an action they can take themselves
--
-- These are built from the RULES and the EVIDENCE, never from the narrating codex.
-- That is the whole point: the codex is exactly what dies during an outage -- it died
-- during the outage that motivated this -- so the fallback body is the one a reader is
-- most likely to see, and it must therefore be the most informative, not the least.
-- This package never remediates anything; it reports, and the actions below are for a
-- human to run.

local function fault_sections(fault)
  if fault == nil then
    return nil
  end
  local where = {}
  if fault.work_item ~= nil then
    table.insert(where, "work item #" .. tostring(fault.work_item))
  end
  if fault.dept ~= nil then
    table.insert(where, "department `" .. tostring(fault.dept) .. "`")
  end
  if fault.queue ~= nil then
    table.insert(where, "queue `" .. tostring(fault.queue) .. "`")
  end

  local impact = {}
  if fault.work_item ~= nil then
    table.insert(
      impact,
      "This work item will NOT produce a pull request. It has exhausted its retries "
        .. "and will not recover on its own."
    )
  else
    table.insert(impact, "Deliveries on this queue are being discarded after their retries run out.")
  end
  if fault.permanent then
    table.insert(
      impact,
      "The failure is marked permanent, so the engine will not redrive it automatically."
    )
  end
  table.insert(
    impact,
    "The session stays alive while the work item is open, so it keeps holding its pod "
      .. "and its CPU/memory reservation without making progress."
  )

  local action = {}
  if fault.log_path ~= nil then
    table.insert(
      action,
      "Read the failing department's own log for the full stack: `" .. tostring(fault.log_path) .. "`."
    )
  end
  for _, line in ipairs({
    "Close and re-open the work item so a fresh proposal is created, or delete the "
      .. "session pod so the session restarts from a clean state.",
    "If it fails again the same way, the cause is upstream of this session -- fix that "
      .. "first, because a retry alone will not clear it.",
  }) do
    table.insert(action, line)
  end

  return {
    log_path = fault.log_path,
    where = #where > 0 and table.concat(where, ", ") or nil,
    impact = impact,
    why = fault.reason,
    attempts = fault.attempts,
    count = fault.count,
    action = action,
  }
end

P.fault_sections = fault_sections

--- The body written when the judge fails, times out, or answers with nothing usable.
--- A missing narrative must never cost a heartbeat: the control plane reads silence
--- as a stalled engine, so the report still goes out with the rules-derived verdict --
--- and, when there is a fault, with the full WHERE / IMPACT / WHY / WHAT TO DO.
function P.fallback_body(verdict, why, fault)
  local out = { "## " .. tostring(verdict.headline), "" }
  local sections = fault_sections(fault)

  if sections ~= nil then
    if sections.where ~= nil then
      table.insert(out, "### Where")
      table.insert(out, sections.where)
      table.insert(out, "")
    end

    table.insert(out, "### What this costs you")
    for _, line in ipairs(sections.impact) do
      table.insert(out, "- " .. line)
    end
    table.insert(out, "")

    table.insert(out, "### Why it failed")
    if sections.why ~= nil then
      table.insert(out, "```")
      table.insert(out, tostring(sections.why))
      table.insert(out, "```")
    elseif sections.log_path ~= nil then
      table.insert(
        out,
        "The engine's own error excerpt is truncated before the cause. The full error is in `"
          .. tostring(sections.log_path) .. "`."
      )
    else
      table.insert(out, "The engine reported no error text for this failure.")
    end
    if sections.attempts ~= nil then
      table.insert(
        out,
        "Retried " .. tostring(sections.attempts) .. " time(s) before being dead-lettered; "
          .. tostring(sections.count) .. " dead letter(s) so far."
      )
    end
    table.insert(out, "")

    table.insert(out, "### How to resolve it")
    for index, line in ipairs(sections.action) do
      table.insert(out, tostring(index) .. ". " .. line)
    end
    table.insert(out, "")
  end

  table.insert(
    out,
    "Status `" .. tostring(verdict.status) .. "` was derived from the evidence in this "
      .. "report's front matter. The narrative summary is unavailable for this window ("
      .. tostring(why) .. "); the verdict, and everything above, are unaffected."
  )
  return table.concat(out, "\n") .. "\n"
end

--- Normalize a codex reply into a report body, or nil when it is not usable.
function P.body_from_reply(reply)
  if type(reply) ~= "string" then
    return nil
  end
  local text = reply:gsub("^%s+", ""):gsub("%s+$", "")
  if text == "" then
    return nil
  end
  return text .. "\n"
end

return P
