local t = fkst.test

local target_path = "packages/github-ratchet-migration-slicer/departments/ratchet_migration_driver/main.lua"
local class_prefix_pattern = "^[a-z0-9][a-z0-9%-]*: [a-z0-9][a-z0-9%-]*:"

return {
  test_ratchet_migration_driver_error_strings_have_class_prefixes = function()
    local source = file.read(target_path)
    local missing = {}
    local line_number = 1
    for line in tostring(source or ""):gmatch("([^\n]*)\n?") do
      local message = line:match("error%(%s*[\"']([^\"']*)[\"']")
      if message ~= nil and not message:match(class_prefix_pattern) then
        table.insert(missing, tostring(line_number) .. ": " .. message)
      end
      line_number = line_number + 1
    end

    t.eq(table.concat(missing, "\n"), "")
  end,
}
