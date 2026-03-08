# Code Coverage Language Server

[![CI](https://github.com/fargies/coverage-lsp/actions/workflows/release.yml/badge.svg)](https://github.com/fargies/coverage-lsp/actions/workflows/release.yml)
[![Crates.io](https://img.shields.io/crates/v/coverage-lsp.svg)](https://crates.io/crates/coverage-lsp)
[![Docs.rs](https://docs.rs/coverage-lsp/badge.svg)](https://docs.rs/coverage-lsp)
[![License](https://img.shields.io/github/license/fargies/coverage-lsp)](LICENSE)


This project implements a Code Coverage
[Language Server](https://microsoft.github.io/language-server-protocol/) that
reads [LCOV](https://lcov.readthedocs.io) coverage reports and exposes the
results through the Language Server Protocol.

Coverage information is surfaced using the
[`textDocument/documentColor`](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_documentColor)
request, allowing editors to visually highlight covered and uncovered lines.

## Zed Editor Configuration

The server can be configured through the following settings.

### `hit`

CSS color used to highlight **covered lines**.

- Accepts any valid [CSS Color Level 4](https://www.w3.org/TR/css-color-4/) value.
- Set to `null` to disable highlighting for covered lines.
- Parsed using [`csscolorparser`](https://crates.io/crates/csscolorparser).
- Default: `rgba(0%,100%,0%,10%)`

### `miss`

CSS color used to highlight **uncovered lines**.

- Accepts any valid [CSS Color Level 4](https://www.w3.org/TR/css-color-4/) value.
- Set to `null` to disable highlighting for uncovered lines.
- Parsed using [`csscolorparser`](https://crates.io/crates/csscolorparser).
- Default: `rgba(100%,0%,0%,10%)`

### `interval`

Interval used to check whether the coverage file has changed.

- Parsed using [`humantime-serde`](https://crates.io/crates/humantime-serde).
- Example values: `3s`, `20s`, `1m`
- Default: `3s`

### `lcov_file`

Path to the [LCOV](https://lcov.readthedocs.io) coverage file to load.

- If not specified, the server searches the workspace and uses the first
  `*.info` file it finds.

## Example Configuration

```json
{
  "lsp": {
    "coverage-lsp": {
      "settings": {
        "hit": null,
        "miss": "#FFAA0020",
        "interval": "20s",
        "lcov_file": "./build/lcov.info"
      }
    }
  }
}
```
