-- todos.lua: an example of a plugin popup for ferra.
--
-- <Space>T lists the TODO, FIXME and XXX comments in the current file in
-- a popup. Move with j/k, <CR> jumps to the one selected, q closes it.
-- It also adds a "TODOs" view to the sidebar (at the bottom of this file).
--
-- Install it by copying (or symlinking) this file into
-- ~/.config/ferra/plugins/.

-- The rows of the TODOs found when the popup opened, so <CR> knows
-- where to go.
local found = {}

local function find_todos(ctx)
  local rows, lines = {}, {}
  for row = 1, ctx:line_count() do
    local text = ctx:line(row)
    -- The first of the tags on the line, if any.
    local at, tag
    for _, name in ipairs({ "TODO", "FIXME", "XXX" }) do
      local i = text:find(name, 1, true)
      if i and (not at or i < at) then
        at, tag = i, name
      end
    end
    if tag then
      local rest = text:sub(at + #tag):gsub("^:?%s*", "")
      table.insert(rows, row)
      table.insert(lines, {
        -- Each line is a list of spans: text and a theme scope to draw it.
        { string.format("%4d  ", row), "ui.linenr" },
        { tag, tag == "TODO" and "ui.popup.title" or "ui.statusline.diagnostic.warning" },
        { "  " .. rest },
      })
    end
  end
  return rows, lines
end

editor.command("todos", "List the TODO comments in this file, to jump to one", function(ctx)
  local rows, lines = find_todos(ctx)
  if #rows == 0 then
    ctx:message("No TODOs in this file")
    return
  end
  found = rows
  local here = ctx:cursor()
  -- Start on the first TODO at or after the cursor.
  local selected = 1
  for i, row in ipairs(rows) do
    if row >= here then
      selected = i
      break
    end
  end
  ctx:popup({
    name = "todos",
    title = "TODOs",
    lines = lines,
    footer = "⏎ go  q close",
    select = true,
    selected = selected,
    keys = { ["<CR>"] = "todos_go" },
  })
end)

editor.command("todos_go", "Go to the TODO selected in the popup", function(ctx)
  local row = found[ctx:popup_selected() or 0]
  ctx:popup_close()
  if row then
    ctx:set_cursor(row, 1)
    ctx:run("goto_first_non_blank")
  end
end)

editor.map("normal", "<Space>T", "todos")

-- The same list as a view in the sidebar (<Tab> in the sidebar to reach
-- it, or :sidebar_view todos), kept up to date as the file changes.
local view_rows = {}

editor.view("todos", {
  title = "TODOs",
  lines = function(ctx)
    local rows, lines = find_todos(ctx)
    view_rows = rows
    return lines
  end,
  keys = { ["<CR>"] = "todos_view_go" },
  refresh = { "buffer-changed" },
})

editor.command("todos_view_go", "Go to the TODO selected in the sidebar", function(ctx)
  local row = view_rows[ctx:view_selected() or 0]
  if row then
    -- Back to the editor (the sidebar stays open), at the TODO.
    ctx:run("sidebar_focus")
    ctx:set_cursor(row, 1)
    ctx:run("goto_first_non_blank")
  end
end)
