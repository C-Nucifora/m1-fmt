# Neovim integration

> **Using more than one M1 tool?** The recommended way to set up M1 in Neovim is
> the unified [nvim-m1](https://github.com/C-Nucifora/nvim-m1) plugin, which wires
> tree-sitter, `m1-lsp`, `m1-fmt`, and `m1-lint` together behind a single `setup`
> call. The standalone plugin below configures **only `m1-fmt`** — use it if you
> want formatting on its own.

`m1-fmt` ships as a lazy.nvim plugin with built-in conform.nvim support.

## Requirements

- [lazy.nvim](https://github.com/folke/lazy.nvim)
- [conform.nvim](https://github.com/stevearc/conform.nvim)
- Rust toolchain (`cargo`) available on `$PATH`

## Installation

```lua
{
  'C-Nucifora/m1-fmt',
  build = 'cargo build --release',
  dependencies = { 'stevearc/conform.nvim' },
  config = function()
    require('m1_fmt').setup({})
  end,
}
```

The `build` step compiles the binary to `target/release/m1-fmt` inside the plugin directory. The plugin resolves the binary path at runtime relative to its own location, so no additional configuration is needed.

## How it works

- `plugin/m1_fmt.lua` registers the `.m1scr` filetype on startup.
- `lua/m1_fmt/init.lua` registers a `m1_fmt` formatter with conform.nvim and maps it to the `m1scr` filetype.
- The formatter reads source from stdin and writes formatted output to stdout (`stdin = true`).

## Custom options

Pass overrides via the `setup` call:

```lua
require('m1_fmt').setup({
  formatter = {
    args = { '--max-blank-lines', '1' },
  },
  conform = {
    format_on_save = { timeout_ms = 500 },
  },
})
```

`opts.formatter` is merged into the conform formatter definition; `opts.conform` is merged into the `conform.setup` call.
