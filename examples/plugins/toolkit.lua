-- toolkit.lua: a small example plugin for terminal-editor.
--
-- Install it by copying (or symlinking) this file into
-- ~/.config/terminal-editor/plugins/. Every command it adds shows up in the
-- command palette (<Space>p) and can be run with :name.
--
-- Rows and columns in the `ctx` API start at 1.

-- Insert a "TODO:" comment above the cursor, matching the line's indent.
editor.command("todo", "Insert a TODO comment above the cursor", function(ctx)
  local row = ctx:cursor()
  local indent = ctx:line(row):match("^%s*")
  local comment = ctx.args[1] or "//"
  ctx:insert_line(row, indent .. comment .. " TODO: ")
  -- Put the cursor at the end of the new comment, ready to type.
  ctx:set_cursor(row, #indent + #comment + 8)
  ctx:run("append_at_line_end")
end)

-- Duplicate the current line below itself ([count] copies).
editor.command("duplicate_line", "Duplicate the current line [count] times", function(ctx)
  local row, col = ctx:cursor()
  local text = ctx:line(row)
  for _ = 1, ctx.count or 1 do
    ctx:insert_line(row + 1, text)
  end
  ctx:set_cursor(row + 1, col)
end)

-- Sort the lines of the paragraph around the cursor (lines between blanks).
editor.command("sort_paragraph", "Sort the lines of the current paragraph", function(ctx)
  local row = ctx:cursor()
  if ctx:line(row) == "" then
    -- Level 0 leaves out Lua's "file:line:" prefix: this message is for
    -- the user, not a bug report.
    error("not in a paragraph", 0)
  end
  local first, last = row, row
  while first > 1 and ctx:line(first - 1) ~= "" do first = first - 1 end
  while last < ctx:line_count() and ctx:line(last + 1) ~= "" do last = last + 1 end

  local lines = {}
  for r = first, last do lines[#lines + 1] = ctx:line(r) end
  table.sort(lines)
  for i, text in ipairs(lines) do
    if ctx:line(first + i - 1) ~= text then
      ctx:set_line(first + i - 1, text)
    end
  end
  ctx:message(("Sorted %d lines"):format(#lines))
end)

-- Count lines, words and characters.
editor.command("word_count", "Show line, word and character counts", function(ctx)
  local text = ctx:text()
  local words = 0
  for _ in text:gmatch("%S+") do words = words + 1 end
  ctx:message(("%d lines, %d words, %d chars"):format(ctx:line_count(), words, #text))
end)

-- Insert today's date at the cursor.
editor.command("insert_date", "Insert today's date (YYYY-MM-DD)", function(ctx)
  ctx:insert(os.date("%Y-%m-%d"))
end)

editor.map("normal", "<Space>t", "todo")
editor.map("normal", "<Space>d", "duplicate_line")
editor.map("normal", "<Space>s", "sort_paragraph")

-- Say which file was just saved, and how big it is now.
editor.on("after-save", function(ctx, event)
  local name = event.path:match("[^/]+$")
  ctx:message(("Saved %s (%d lines)"):format(name, ctx:line_count()))
end)
