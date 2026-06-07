# Yazelix Helix Fork

Yazelix Helix is an active Yazelix fork of Helix's Steel-enabled editor line.
It is usable as a standalone editor without the full Yazelix workspace.

| Field | Value |
| --- | --- |
| Upstream project | Helix, with Yazelix tracking the active Helix Steel line |
| Fork category | Active fork |
| Why this fork exists | Yazelix needs a Steel-enabled Helix package with a config-directory override and editor action bridge hooks while keeping `~/.config/helix` untouched |
| Current Yazelix delta | `--config-dir`, reusable Steel plugin defaults, and Yazelix bridge hooks behind explicit runtime flags |
| Non-goals | This fork does not own Yazelix workspace orchestration, generated Zellij/Yazi configs, or main-repo release policy |
| Standalone support | Supported: users can run this fork directly as a Helix editor; Yazelix-only bridge behavior stays behind explicit runtime/session flags |
| Upstream sync cadence | Monthly or before Helix-sensitive Yazelix releases |
| Upstreaming/removal gate | Upstream reusable pieces when accepted, but keep the fork while its standalone Steel/defaults value is higher than upstream Helix plus main-repo adapters |

Fork notes and the packaged Steel defaults contract live in
[YAZELIX.md](./YAZELIX.md). Main Yazelix fork policy lives in
[Fork and child-repo maintenance](https://github.com/luccahuguet/yazelix/blob/main/docs/contracts/fork_child_repo_maintenance.md).

## Upstream Helix README

<div align="center">

<h1>
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="logo_dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="logo_light.svg">
  <img alt="Helix" height="128" src="logo_light.svg">
</picture>
</h1>

[![Build status](https://github.com/helix-editor/helix/actions/workflows/build.yml/badge.svg)](https://github.com/helix-editor/helix/actions)
[![GitHub Release](https://img.shields.io/github/v/release/helix-editor/helix)](https://github.com/helix-editor/helix/releases/latest)
[![Documentation](https://shields.io/badge/-documentation-452859)](https://docs.helix-editor.com/)
[![GitHub contributors](https://img.shields.io/github/contributors/helix-editor/helix)](https://github.com/helix-editor/helix/graphs/contributors)
[![Matrix Space](https://img.shields.io/matrix/helix-community:matrix.org)](https://matrix.to/#/#helix-community:matrix.org)

</div>

![Screenshot](./screenshot.png)

A [Kakoune](https://github.com/mawww/kakoune) / [Neovim](https://github.com/neovim/neovim) inspired editor, written in Rust.

The editing model is very heavily based on Kakoune; during development I found
myself agreeing with most of Kakoune's design decisions.

For more information, see the [website](https://helix-editor.com) or
[documentation](https://docs.helix-editor.com/).

All shortcuts/keymaps can be found [in the documentation on the website](https://docs.helix-editor.com/keymap.html).

[Troubleshooting](https://github.com/helix-editor/helix/wiki/Troubleshooting)

# Features

- Vim-like modal editing
- Multiple selections
- Built-in language server support
- Smart, incremental syntax highlighting and code editing via tree-sitter

Although it's primarily a terminal-based editor, I am interested in exploring
a custom renderer (similar to Emacs) using wgpu.

Note: Only certain languages have indentation definitions at the moment. Check
`runtime/queries/<lang>/` for `indents.scm`.

# Installation

[Installation documentation](https://docs.helix-editor.com/install.html).

[![Packaging status](https://repology.org/badge/vertical-allrepos/helix-editor.svg?exclude_unsupported=1)](https://repology.org/project/helix-editor/versions)

# Contributing

Contributing guidelines can be found [here](./docs/CONTRIBUTING.md).

# Getting help

Your question might already be answered on the [FAQ](https://github.com/helix-editor/helix/wiki/FAQ).

Discuss the project on the community [Matrix Space](https://matrix.to/#/#helix-community:matrix.org) (make sure to join `#helix-editor:matrix.org` if you're on a client that doesn't support Matrix Spaces yet).

# Credits

Thanks to [@jakenvac](https://github.com/jakenvac) for designing the logo!
