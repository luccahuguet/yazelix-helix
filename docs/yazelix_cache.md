# Yazelix Helix binary cache

This fork publishes the `.#packages.x86_64-linux.yazelix_helix` package to the public Yazelix Cachix cache:

```conf
extra-substituters = https://yazelix.cachix.org
extra-trusted-public-keys = yazelix.cachix.org-1:ZgxIjQvaP0VTWL8Racx27mpUNzDJ97xC2y7QWYjmGNM=
```

The `.github/workflows/cachix.yml` workflow builds the package on pull requests and publishes successful `main` or manual builds with the repository `CACHIX_AUTH_TOKEN` secret. It also trusts the upstream Helix Cachix cache for shared dependencies.

Yazelix main consumes this package through its locked `yazelixHelix` flake input. Cache misses are expected for unpublished revisions and fall back to a normal source build.

Helix tree-sitter grammar sources are locked in `grammar_sources.lock.json` and fetched through fixed-output `fetchFromGitHub` / `fetchgit` derivations instead of eval-time `builtins.fetchTree`, so cold evaluation stays local and grammar source builds can substitute from cache.

When `languages.toml` grammar sources change, regenerate the lock with:

```bash
cargo xtask grammar-lock update
cargo xtask grammar-lock validate
```

To check whether the current public child output is available:

```bash
out="$(nix eval --raw github:luccahuguet/yazelix-helix#packages.x86_64-linux.yazelix_helix.outPath)"
nix path-info --store https://yazelix.cachix.org "$out"
```
