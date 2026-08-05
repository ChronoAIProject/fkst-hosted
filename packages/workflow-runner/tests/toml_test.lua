local toml = require("core.toml")
local t = fkst.test

local DEFINITION = table.concat({
  "# A three-step sourcing pipeline.",
  'description = "source, score, publish"',
  "",
  "[[step]]",
  'id = "scrape"',
  'kind = "run"',
  'command = ["python3", ".fkst/workflows/sourcing/scrape.py", "--role", "{{ role }}"]',
  "timeout_secs = 600",
  "",
  "[[step]]",
  'id = "score"',
  'kind = "task"',
  'prompt = "Score each entry in candidates.json."',
  "",
  "[[step]]",
  'id = "publish"',
  'kind = "run"',
  'command = ["python3", "publish.py"]',
  "enabled = true",
}, "\n")

return {
  test_a_workflow_definition_decodes = function()
    local document, err = toml.decode(DEFINITION)
    t.is_nil(err)
    t.eq(document.description, "source, score, publish")
    t.eq(#document.step, 3)
    t.eq(document.step[1].id, "scrape")
    t.eq(document.step[1].timeout_secs, 600)
    t.eq(#document.step[1].command, 4)
    t.eq(document.step[1].command[4], "{{ role }}")
    t.eq(document.step[2].kind, "task")
    t.eq(document.step[3].enabled, true)
  end,

  test_comments_and_blank_lines_are_ignored = function()
    local document = toml.decode('# only a comment\n\n\n[[step]]\nid = "a"\n')
    t.eq(#document.step, 1)
  end,

  test_string_escapes_decode = function()
    local document = toml.decode('[[step]]\nprompt = "a \\"quoted\\" word\\nline\\ttab\\\\end"\n')
    t.eq(document.step[1].prompt, 'a "quoted" word\nline\ttab\\end')
  end,

  test_an_empty_array_decodes = function()
    local document = toml.decode('[[step]]\ncommand = []\n')
    t.eq(#document.step[1].command, 0)
  end,

  test_an_array_may_span_lines = function()
    -- A long argv is the normal case for a `run` step, and one element per line
    -- is how anyone would naturally format it.
    local document, err = toml.decode(table.concat({
      "[[step]]",
      'id = "scrape"',
      "command = [",
      '  "python3",',
      '  "scrape.py",',
      '  "--role",',
      '  "{{ role }}",',
      "]",
      "timeout_secs = 600",
    }, "\n"))
    t.is_nil(err)
    t.eq(#document.step[1].command, 4)
    t.eq(document.step[1].command[4], "{{ role }}")
    -- The key AFTER the array must still be read: a fold that swallowed the
    -- following line would silently drop a step's timeout.
    t.eq(document.step[1].timeout_secs, 600)
  end,

  test_a_bracket_inside_a_string_does_not_open_an_array = function()
    local document, err = toml.decode('[[step]]\nprompt = "signals: [\\"a\\", \\"b\\"]"\nid = "x"\n')
    t.is_nil(err)
    t.eq(document.step[1].prompt, 'signals: ["a", "b"]')
    t.eq(document.step[1].id, "x")
  end,

  test_a_bracket_inside_a_multi_line_prompt_does_not_open_an_array = function()
    -- Arrays fold AFTER strings precisely so this holds.
    local document, err = toml.decode(
      '[[step]]\nprompt = """\nemit [{"id": 1}]\nand nothing else\n"""\nid = "score"\n'
    )
    t.is_nil(err)
    t.eq(document.step[1].prompt, 'emit [{"id": 1}]\nand nothing else\n')
    t.eq(document.step[1].id, "score")
  end,

  test_unsupported_syntax_is_refused_with_a_line_number = function()
    -- Refusing is the point. A definition using a TOML feature this reader does
    -- not implement would otherwise be silently misread — and a misread
    -- definition runs the wrong commands.
    for _, case in ipairs({
      { text = '[[step]]\nid = "a"\n[server]\nhost = "x"\n', why = "unsupported table header" },
      { text = "[[step]]\nid = 'single quoted'\n", why = "unsupported value syntax" },
      { text = '[[step]]\nid = "one" "two"\n', why = "trailing content" },
      { text = '[[step]]\nid = "unterminated\n', why = "unterminated string" },
      { text = "[[step]]\nid = -1\n", why = "unsupported value syntax" },
      { text = "[[step]]\nid = 1.5\n", why = "unsupported value syntax" },
      { text = '[[step]]\nid = "a"\nid = "b"\n', why = "duplicate key" },
      { text = "[[step]]\nnot a key value line\n", why = "unsupported syntax" },
      { text = '[[step]]\nid = "bad \\q escape"\n', why = "unsupported escape" },
      { text = '[[step]]\ncommand = ["a",\n', why = "unterminated array" },
    }) do
      local document, err = toml.decode(case.text)
      t.is_nil(document)
      t.is_true(
        err:find(case.why, 1, true) ~= nil,
        ("expected %q in %q"):format(case.why, tostring(err))
      )
    end
  end,

  test_the_line_number_points_at_the_offending_line = function()
    local _, err = toml.decode('[[step]]\nid = "a"\nbroken line here\n')
    t.is_true(err:find("line 3", 1, true) ~= nil, err)
  end,

  test_a_non_string_input_is_refused = function()
    local document, err = toml.decode(nil)
    t.is_nil(document)
    t.is_true(err ~= nil)
  end,

  -- ---- multi-line strings ------------------------------------------------

  test_a_multi_line_string_keeps_its_shape = function()
    -- A task step's prompt is prose. Forcing it onto one physical line makes
    -- definitions unreadable and invites authors to reach for a TOML feature
    -- this reader does not have.
    local document, err = toml.decode(
      '[[step]]\nprompt = """\nfirst line\n\nthird line\n"""\nid = "score"\n'
    )
    t.is_nil(err)
    t.eq(document.step[1].prompt, "first line\n\nthird line\n")
    t.eq(document.step[1].id, "score")
  end,

  test_multi_line_content_is_literal = function()
    -- No escape processing inside `"""`: prose is far likelier to contain a
    -- stray backslash than an intended escape, and a bare quote needs no
    -- escaping because only `"""` terminates.
    local document = toml.decode('[[step]]\nprompt = """\nsay "hi" and a\\path\n"""\n')
    t.eq(document.step[1].prompt, 'say "hi" and a\\path\n')
  end,

  test_a_single_line_triple_quoted_value_works = function()
    local document = toml.decode('[[step]]\nprompt = """one line"""\n')
    t.eq(document.step[1].prompt, "one line")
  end,

  test_an_unterminated_multi_line_string_is_refused = function()
    local document, err = toml.decode('[[step]]\nprompt = """\nnever closed\n')
    t.is_nil(document)
    t.is_true(err:find("unterminated multi-line string", 1, true) ~= nil, tostring(err))
  end,

  test_a_single_line_string_still_processes_escapes = function()
    -- The two forms differ deliberately; this pins that the single-line form was
    -- not changed by adding the multi-line one.
    local document = toml.decode('[[step]]\nid = "a\\nb"\n')
    t.eq(document.step[1].id, "a\nb")
  end,
}
