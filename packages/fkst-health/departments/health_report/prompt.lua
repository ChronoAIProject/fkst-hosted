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
function P.context(verdict, session_id)
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
function P.build(verdict, context_path)
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
    "Hard constraints:",
    "- Do not assert any progress, failure, or cause that is not in the evidence.",
    "- Do not restate a different status, and do not hedge the given one.",
    "- If the evidence is thin, say plainly that little could be observed.",
    "- Do not invent issue numbers, file names, commits, or error messages.",
    "- Output markdown prose only: no front matter, no headings, no code fences.",
  }, "\n")
end

--- The body written when the judge fails, times out, or answers with nothing usable.
--- A missing narrative must never cost a heartbeat: the control plane reads silence
--- as a stalled engine, so the report still goes out with the rules-derived verdict.
function P.fallback_body(verdict, why)
  return table.concat({
    "## " .. tostring(verdict.headline),
    "",
    "Status `" .. tostring(verdict.status) .. "` was derived from the evidence listed in this",
    "report's front matter. The narrative summary is unavailable for this window ("
      .. tostring(why)
      .. "); the verdict and its evidence are unaffected.",
  }, "\n") .. "\n"
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
