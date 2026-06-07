# Agent Guidelines

Shared Yazelix agent workflow and release policy live in the main repo:

- https://github.com/luccahuguet/yazelix/blob/main/AGENTS.md
- In sibling local checkouts, read `../yazelix/AGENTS.md` first

Only Yazelix Helix-specific guidance belongs here.

## Local Scope

- This repo is the Yazelix-owned Helix fork, not the main Yazelix workspace.
- Keep the fork delta small and reviewable: config-directory support, reusable Steel defaults, and explicit Yazelix bridge hooks.
- Keep standalone Helix behavior usable without Yazelix.
- Do not put generated Zellij, Yazi, or main Yazelix runtime policy in this repo.

## Local Commands

- `cargo fmt --all -- --check`
- `cargo check -p helix-term`
- `cargo test -p helix-term yazelix_bridge --features steel`
- `nix build .#yazelix_helix --no-link`

## Integration Notes

Main Yazelix consumes this repo through its flake input and owns the managed editor session policy. For coupled runtime changes, publish this child commit before updating the main repo lock.
