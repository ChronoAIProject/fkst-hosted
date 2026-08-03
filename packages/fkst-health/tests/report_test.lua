-- Tests for the pure rendering layer: the contract filename and the TOML front
-- matter. Everything asserted here is a property the control plane's parser
-- (fkst-hosted backend/src/session_health/{report,naming}.rs) depends on, so a
-- failure here means every report on the platform stops being indexable.
local report = require("report")
local t = fkst.test

local session = "8f2c1d64-0a1b-4c2d-8e3f-0123456789ab"
local at = 1785650400 -- 2026-08-02T06:00:00Z

local function front_matter(text)
  local body_at = text:find("\n+++\n", 4, true)
  t.is_true(body_at ~= nil, "front matter is not terminated: " .. text:sub(1, 120))
  return text:sub(5, body_at)
end

local function document(overrides)
  local doc = {
    session_id = session,
    namespace = "chronoai-fkst",
    generated_at = at,
    window_start = at - 600,
    status = "stalled",
    headline = "No movement in 10m with 3 work items open",
    confidence = "high",
    evidence = { { key = "deliveries_completed_delta", value = "0" } },
    work_items = { { number = 812, state = "open", progress = "none" } },
    body = "## What this session is doing\n\nNothing observable.\n",
  }
  for key, value in pairs(overrides or {}) do
    if value == false then
      doc[key] = nil
    else
      doc[key] = value
    end
  end
  return doc
end

return {
  -- ---- filenames ------------------------------------------------------------
  test_filename_carries_the_namespace_and_a_colon_free_utc_stamp = function()
    local name = report.filename("chronoai-fkst", session, at)
    t.eq(name, "chronoai-fkst-" .. session .. "-health-agent-status-report-20260802-060000.md")
    t.is_true(name:find(":") == nil, name)
  end,

  -- The parser splits the prefix by anchoring on the trailing 36-character UUID, so
  -- an absent namespace must drop the segment AND its joining hyphen. A placeholder
  -- or a leading hyphen would be read back as part of the session id.
  test_filename_omits_the_namespace_segment_and_its_hyphen_when_unset = function()
    local expected = session .. "-health-agent-status-report-20260802-060000.md"
    t.eq(report.filename(nil, session, at), expected)
    t.eq(report.filename("", session, at), expected)
    t.eq(report.filename("   ", session, at), expected)
    t.is_true(expected:sub(1, 1) ~= "-", expected)
  end,

  -- A namespace or session id carrying a path separator would let a report escape its
  -- directory and its object-key prefix; the parser rejects such a name outright, so
  -- the producer sanitizes rather than emitting something nothing downstream indexes.
  test_filename_segments_are_sanitized_against_traversal = function()
    local name = report.filename("../../etc", "a/b\\c", at)
    t.is_true(name:find("/") == nil, name)
    t.is_true(name:find("\\") == nil, name)
    t.is_true(name:find("%.%.") == nil, name)
    t.is_true(name:sub(1, 1) ~= ".", name)
    -- Still a name the control plane will index: the marker and stamp survive.
    t.is_true(name:find("-health-agent-status-report-20260802-060000.md", 1, true) ~= nil, name)
  end,

  test_stamp_is_utc_and_sorts_chronologically = function()
    t.eq(report.stamp(at), "20260802-060000")
    t.eq(report.rfc3339(at), "2026-08-02T06:00:00Z")
    t.is_true(report.stamp(at) < report.stamp(at + 1), "stamps must sort by time")
  end,

  -- ---- front matter ---------------------------------------------------------
  test_render_emits_toml_front_matter_fenced_by_plus_signs = function()
    local text = report.render(document(), 600)
    t.eq(text:sub(1, 4), "+++\n")
    t.is_true(text:find("\n+++\n", 4, true) ~= nil, "closing fence missing")
    -- YAML fences would be silently unparseable; the contract is TOML.
    t.is_true(text:sub(1, 4) ~= "---\n", "front matter must not be YAML-fenced")
  end,

  test_render_carries_every_required_v1_field = function()
    local matter = front_matter(report.render(document(), 600))
    for _, needle in ipairs({
      "fkst_health_report = 1",
      'session_id = "' .. session .. '"',
      'namespace = "chronoai-fkst"',
      'producer = "fkst-health@0.1.0"',
      'generated_at = "2026-08-02T06:00:00Z"',
      'window_start = "2026-08-02T05:50:00Z"',
      "expected_interval_secs = 600",
      'status = "stalled"',
      'confidence = "high"',
    }) do
      t.is_true(matter:find(needle, 1, true) ~= nil, "missing: " .. needle .. "\n" .. matter)
    end
  end,

  -- TOML requires every scalar key to precede any table or array-of-tables. A
  -- producer that renders [[evidence]] before `headline` emits a document that is not
  -- valid TOML at all, and the parser rejects the whole file.
  test_every_scalar_key_precedes_the_arrays_of_tables = function()
    local matter = front_matter(report.render(document(), 600))
    local first_table = matter:find("[[evidence]]", 1, true)
    t.is_true(first_table ~= nil, matter)
    for _, key in ipairs({
      "fkst_health_report",
      "session_id",
      "namespace",
      "producer",
      "generated_at",
      "window_start",
      "expected_interval_secs",
      "status",
      "headline",
      "confidence",
    }) do
      local at_key = matter:find("\n" .. key .. " = ", 1, true) or matter:find(key .. " = ", 1, true)
      t.is_true(at_key ~= nil, "missing scalar " .. key)
      t.is_true(at_key < first_table, key .. " must precede [[evidence]]")
    end
    t.is_true(matter:find("[[work_items]]", 1, true) > first_table, "work_items must follow evidence")
  end,

  test_optional_fields_are_omitted_rather_than_emitted_empty = function()
    local matter = front_matter(report.render(document({
      namespace = false,
      window_start = false,
      confidence = false,
    }), 600))
    t.eq(matter:find("namespace", 1, true), nil)
    t.eq(matter:find("window_start", 1, true), nil)
    t.eq(matter:find("confidence", 1, true), nil)
    t.is_true(matter:find("session_id", 1, true) ~= nil, matter)
  end,

  test_string_values_are_escaped_so_a_quote_cannot_break_the_document = function()
    local matter = front_matter(report.render(document({
      headline = 'he said "wedged" on\nline two \\ here',
    }), 600))
    t.is_true(matter:find('\\"wedged\\"', 1, true) ~= nil, matter)
    t.is_true(matter:find("\\\\", 1, true) ~= nil, matter)
    -- The headline is one physical line; a raw newline would end the TOML key.
    local line = matter:match('headline = "[^\n]*"')
    t.is_true(line ~= nil, matter)
  end,

  -- ---- bounds ---------------------------------------------------------------
  -- An over-long headline is TRUNCATED, never a reason to drop the report: losing the
  -- report costs a heartbeat, and the control plane reads silence as a stalled engine.
  test_an_over_long_headline_is_truncated_not_dropped = function()
    local text = report.render(document({ headline = string.rep("x", 5000) }), 600)
    local matter = front_matter(text)
    local headline = matter:match('headline = "(.-)"\n')
    t.is_true(headline ~= nil, matter)
    t.is_true(#headline <= report.headline_character_ceiling, tostring(#headline))
    t.is_true(matter:find("session_id", 1, true) ~= nil, "the report is still emitted")
  end,

  test_evidence_and_work_items_are_capped_at_the_contract_bounds = function()
    local evidence, items = {}, {}
    for index = 1, 500 do
      table.insert(evidence, { key = "k" .. index, value = index })
      table.insert(items, { number = index, state = "open", progress = "none" })
    end
    local matter = front_matter(report.render(document({ evidence = evidence, work_items = items }), 600))
    local evidence_count, item_count = 0, 0
    for _ in matter:gmatch("%[%[evidence%]%]") do
      evidence_count = evidence_count + 1
    end
    for _ in matter:gmatch("%[%[work_items%]%]") do
      item_count = item_count + 1
    end
    t.eq(evidence_count, report.evidence_entry_ceiling)
    t.eq(item_count, report.work_item_ceiling)
  end,

  test_malformed_entries_are_dropped_rather_than_rendered = function()
    local matter = front_matter(report.render(document({
      evidence = { { key = "", value = "x" }, 7, { value = "no key" }, { key = "kept", value = "1" } },
      work_items = { { state = "open" }, { number = "nope" }, { number = 5, state = "open", progress = "none" } },
    }), 600))
    local evidence_count, item_count = 0, 0
    for _ in matter:gmatch("%[%[evidence%]%]") do
      evidence_count = evidence_count + 1
    end
    for _ in matter:gmatch("%[%[work_items%]%]") do
      item_count = item_count + 1
    end
    t.eq(evidence_count, 1)
    t.eq(item_count, 1)
    t.is_true(matter:find('key = "kept"', 1, true) ~= nil, matter)
    t.is_true(matter:find("number = 5", 1, true) ~= nil, matter)
  end,

  -- The collector SKIPS a file larger than its ceiling, so a pathological narrative
  -- must be clipped rather than allowed to cost the heartbeat.
  test_the_rendered_report_stays_within_the_collector_ceiling = function()
    local text = report.render(document({ body = string.rep("narrative ", 200000) }), 600)
    t.is_true(#text <= report.report_byte_ceiling, tostring(#text))
    t.is_true(text:sub(1, 4) == "+++\n", "front matter survives the clip")
  end,

  -- ---- totality -------------------------------------------------------------
  test_render_is_total_over_malformed_documents = function()
    for _, case in ipairs({
      { n = 1 },
      { n = 2, doc = {} },
      { n = 3, doc = { evidence = "nope", work_items = 7 } },
      { n = 4, doc = { generated_at = "not a number", status = nil } },
      { n = 5, doc = { generated_at = 0 / 0 } },
    }) do
      local ok, text = pcall(report.render, case.doc, 600)
      t.is_true(ok, "case " .. tostring(case.n) .. " errored: " .. tostring(text))
      t.eq(text:sub(1, 4), "+++\n")
      t.is_true(text:find("fkst_health_report = 1", 1, true) ~= nil, "case " .. tostring(case.n))
    end
  end,
}
