local t = fkst.test

local function package_root()
  local source = package.searchpath("tests.ratchet_migration_extraction_test", package.path)
  return source:match("(.+)/tests/ratchet_migration_extraction_test%.lua$")
end

local function exists(path)
  local handle = io.open(path, "r")
  if handle == nil then
    return false
  end
  handle:close()
  return true
end

return {
  test_github_devloop_no_longer_owns_ratchet_migration_slicer = function()
    local root = package_root()

    t.eq(exists(root .. "/departments/ratchet_migration_driver/main.lua"), false)
    t.eq(exists(root .. "/raisers/ratchet_migration_slicer.lua"), false)
  end,
}
