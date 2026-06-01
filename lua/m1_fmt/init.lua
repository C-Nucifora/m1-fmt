local M = {}

function M.setup(opts)
  opts = opts or {}
  local plugin_dir = vim.fn.fnamemodify(debug.getinfo(1, "S").source:sub(2), ":h:h:h")
  local bin = plugin_dir .. "/target/release/m1-fmt"

  local ok, conform = pcall(require, "conform")
  if not ok then
    vim.notify("m1_fmt: conform.nvim not found", vim.log.levels.WARN)
    return
  end

  -- Don't register a formatter pointing at a binary that isn't built yet —
  -- conform would otherwise fail at format-time with an opaque error.
  if vim.fn.executable(bin) == 0 then
    vim.notify(
      "m1_fmt: formatter binary not found at "
        .. bin
        .. " — run `cargo build --release` in the plugin directory",
      vim.log.levels.WARN
    )
    return
  end

  -- Let opts.formatter override extras (e.g. args), but `command` and
  -- `stdin` are non-negotiable — m1-fmt is our binary and always reads stdin,
  -- so force them last rather than let an override silently drop them.
  local formatter = vim.tbl_deep_extend("force", { args = {} }, opts.formatter or {}, {
    command = bin,
    stdin = true,
  })

  if opts.conform then
    -- Caller opted into plugin-managed conform config: merge our defaults
    -- (the m1_fmt formatter + m1scr mapping) with everything they passed
    -- (format_on_save, notify_on_error, …) and run conform.setup once, so all
    -- of opts.conform takes effect — not just formatters_by_ft.
    local cfg = vim.tbl_deep_extend("force", {
      formatters = { m1_fmt = formatter },
      formatters_by_ft = { m1scr = { "m1_fmt" } },
    }, opts.conform)
    conform.setup(cfg)
  else
    -- Default: don't call conform.setup() (it would clobber a conform config
    -- the user set up themselves); register directly into the live tables.
    conform.formatters.m1_fmt = formatter
    conform.formatters_by_ft =
      vim.tbl_extend("force", conform.formatters_by_ft or {}, { m1scr = { "m1_fmt" } })
  end
end

return M
