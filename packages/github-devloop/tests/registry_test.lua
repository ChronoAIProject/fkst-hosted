local registry = require("core.registry")
local t = fkst.test

local function with_modules(modules, fn)
  local old_preload = {}
  local old_loaded = {}
  for name, loader in pairs(modules) do
    old_preload[name] = package.preload[name]
    old_loaded[name] = package.loaded[name]
    package.preload[name] = loader
    package.loaded[name] = nil
  end
  local ok, err = pcall(fn)
  for name in pairs(modules) do
    package.preload[name] = old_preload[name]
    package.loaded[name] = old_loaded[name]
  end
  if not ok then
    error(err)
  end
end

local function expect_error_contains(fn, needle)
  local ok, err = pcall(fn)
  t.eq(ok, false)
  t.is_true(tostring(err):find(needle, 1, true) ~= nil, tostring(err))
end

return {
  test_indexed_map_loads_explicit_sorted_entries = function()
    with_modules({
      ["tests.fake_registry.index"] = function()
        return {
          { module = "first_entry", key = "first-entry" },
          { module = "second_entry", key = "second-entry" },
        }
      end,
      ["tests.fake_registry.first_entry"] = function()
        return { name = "first-entry", value = "a" }
      end,
      ["tests.fake_registry.second_entry"] = function()
        return { name = "second-entry", value = "b" }
      end,
    }, function()
      local loaded = registry.load_indexed_map("tests.fake_registry.index", "name")
      t.eq(loaded["first-entry"].value, "a")
      t.eq(loaded["second-entry"].value, "b")
      t.eq(loaded["first-entry"].name, nil)

      local loaded_again = registry.load_indexed_map("tests.fake_registry.index", "name")
      t.eq(loaded_again["first-entry"].value, "a")
      t.eq(loaded_again["second-entry"].value, "b")
      t.eq(loaded_again["first-entry"].name, nil)
    end)
  end,

  test_indexed_array_rejects_unsorted_index = function()
    with_modules({
      ["tests.unsorted_registry.index"] = function()
        return { "z", "a" }
      end,
    }, function()
      expect_error_contains(function()
        registry.load_indexed_array("tests.unsorted_registry.index", "name")
      end, "not sorted")
    end)
  end,

  test_indexed_array_rejects_duplicate_index_entries = function()
    with_modules({
      ["tests.duplicate_registry.index"] = function()
        return { "a", "a" }
      end,
    }, function()
      expect_error_contains(function()
        registry.load_indexed_array("tests.duplicate_registry.index", "name")
      end, "duplicate registry index entry")
    end)
  end,

  test_indexed_array_rejects_entry_key_mismatch = function()
    with_modules({
      ["tests.mismatch_registry.index"] = function()
        return {
          { module = "entry", key = "expected" },
        }
      end,
      ["tests.mismatch_registry.entry"] = function()
        return { name = "actual" }
      end,
    }, function()
      expect_error_contains(function()
        registry.load_indexed_array("tests.mismatch_registry.index", "name")
      end, "does not match index entry")
    end)
  end,
}
