local M = {}

function M.setup(opts)
  opts = opts or {}
  local plugin_dir = vim.fn.fnamemodify(debug.getinfo(1, 'S').source:sub(2), ':h:h:h')
  local bin = plugin_dir .. '/target/release/m1-fmt'

  local ok, conform = pcall(require, 'conform')
  if not ok then
    vim.notify('m1_fmt: conform.nvim not found', vim.log.levels.WARN)
    return
  end

  conform.formatters.m1_fmt = vim.tbl_deep_extend('force', {
    command = bin,
    stdin = true,
    args = {},
  }, opts.formatter or {})

  conform.setup(vim.tbl_deep_extend('force', {
    formatters_by_ft = { m1scr = { 'm1_fmt' } },
  }, opts.conform or {}))
end

return M
