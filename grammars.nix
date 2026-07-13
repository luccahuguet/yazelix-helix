{
  stdenv,
  lib,
  runCommand,
  fetchgit,
  fetchzip,
  includeGrammarIf ? _: true,
  grammarOverlays ? [],
  ...
}: let
  languagesConfig =
    builtins.fromTOML (builtins.readFile ./languages.toml);
  grammarLock =
    builtins.fromJSON (builtins.readFile ./grammar_sources.lock.json);
  isGitGrammar = grammar:
    builtins.hasAttr "source" grammar
    && builtins.hasAttr "git" grammar.source
    && builtins.hasAttr "rev" grammar.source;
  isGitHubGrammar = grammar: lib.hasPrefix "https://github.com" grammar.source.git;
  isArchiveGrammarUrl = url:
    lib.hasPrefix "https://codeberg.org/" url
    || lib.hasPrefix "https://git.sr.ht/" url;
  archiveGrammarUrl = url: rev: let
    trimmed = lib.removeSuffix "/" url;
  in
    if lib.hasPrefix "https://codeberg.org/" trimmed
    then "${trimmed}/archive/${rev}.tar.gz"
    else if lib.hasPrefix "https://git.sr.ht/" trimmed
    then "${trimmed}/archive/${rev}.tar.gz"
    else throw "grammar source URL has no known archive fetcher: ${url}";
  toGitHubFetcher = url: let
    match = builtins.match "https://github\\.com/([^/]*)/([^/]*)/?" url;
  in {
    owner = builtins.elemAt match 0;
    repo = builtins.elemAt match 1;
  };
  # If `use-grammars.only` is set, use only those grammars.
  # If `use-grammars.except` is set, use all other grammars.
  # Otherwise use all grammars.
  useGrammar = grammar:
    if languagesConfig ? use-grammars.only
    then builtins.elem grammar.name languagesConfig.use-grammars.only
    else if languagesConfig ? use-grammars.except
    then !(builtins.elem grammar.name languagesConfig.use-grammars.except)
    else true;
  grammarsToUse = builtins.filter useGrammar languagesConfig.grammar;
  gitGrammars = builtins.filter isGitGrammar grammarsToUse;
  requireLockEntry = name:
    if !(grammarLock.grammars ? ${name}) then
      throw "grammar_sources.lock.json is missing entry for grammar '${name}'. Run: cargo xtask grammar-lock update"
    else grammarLock.grammars.${name};
  # Match upstream Helix's fetchTree path except where a lock entry requires a
  # sparse checkout, which fetchTree cannot express.
  fetchSparseGitLocked = entry: args:
    fetchgit (args
      // lib.optionalAttrs (entry ? sparseCheckout) {
        inherit (entry) sparseCheckout;
      });
  fetchTreeLocked = entry: args:
    builtins.fetchTree (args // {narHash = entry.hash;});
  fetchGrammarSrc = grammar: let
    entry = requireLockEntry grammar.name;
    grammarRev = grammar.source.rev;
    grammarGit = grammar.source.git;
  in
    if entry.fetcher == "github" then
      let
        gh = toGitHubFetcher grammarGit;
      in
        if entry.owner != gh.owner || entry.repo != gh.repo then
          throw "grammar_sources.lock.json owner/repo mismatch for grammar '${grammar.name}'"
        else if entry.rev != grammarRev then
          throw "grammar_sources.lock.json rev mismatch for grammar '${grammar.name}'"
        else if entry ? sparseCheckout then
          fetchSparseGitLocked entry {
            url = "https://github.com/${entry.owner}/${entry.repo}";
            rev = entry.rev;
            sha256 = entry.hash;
            fetchSubmodules = false;
          }
        else
          fetchTreeLocked entry {
            type = "github";
            inherit (entry) owner repo rev;
          }
    else if entry.fetcher == "git" then
      if entry.url != grammarGit then
        throw "grammar_sources.lock.json url mismatch for grammar '${grammar.name}'"
      else if entry.rev != grammarRev then
        throw "grammar_sources.lock.json rev mismatch for grammar '${grammar.name}'"
      else if isArchiveGrammarUrl grammarGit then
        throw "grammar_sources.lock.json unexpectedly uses raw git fetcher for archive-capable grammar '${grammar.name}'"
      else if entry ? sparseCheckout then
        fetchSparseGitLocked entry {
          url = entry.url;
          rev = entry.rev;
          sha256 = entry.hash;
          fetchSubmodules = false;
        }
      else
        fetchTreeLocked entry {
          type = "git";
          url = entry.url;
          rev = entry.rev;
          ref = grammar.source.ref or "HEAD";
          shallow = true;
        }
    else if entry.fetcher == "archive" then
      if entry.url != grammarGit then
        throw "grammar_sources.lock.json url mismatch for grammar '${grammar.name}'"
      else if entry.rev != grammarRev then
        throw "grammar_sources.lock.json rev mismatch for grammar '${grammar.name}'"
      else if !(isArchiveGrammarUrl grammarGit) then
        throw "grammar_sources.lock.json uses archive fetcher for grammar '${grammar.name}' without a known archive URL"
      else
        fetchzip {
          url = archiveGrammarUrl entry.url entry.rev;
          sha256 = entry.hash;
          stripRoot = true;
        }
    else
      throw "grammar_sources.lock.json has unsupported fetcher '${entry.fetcher}' for grammar '${grammar.name}'";
  buildGrammar = grammar: let
    fetchedSrc = fetchGrammarSrc grammar;
    grammarSrc =
      if builtins.hasAttr "subpath" grammar.source
      then "${fetchedSrc}/${grammar.source.subpath}"
      else fetchedSrc;
  in
    stdenv.mkDerivation {
      # see https://github.com/NixOS/nixpkgs/blob/fbdd1a7c0bc29af5325e0d7dd70e804a972eb465/pkgs/development/tools/parsing/tree-sitter/grammar.nix

      pname = "helix-tree-sitter-${grammar.name}";
      version = grammar.source.rev;

      src = grammarSrc;

      dontUnpack = true;
      dontConfigure = true;

      FLAGS = [
        "-I${grammarSrc}/src"
        "-g"
        "-O3"
        "-fPIC"
        "-fno-exceptions"
        "-Wl,-z,relro,-z,now"
      ];

      SHARED_LIB = grammar.name + stdenv.hostPlatform.extensions.sharedLibrary;

      buildPhase = ''
        runHook preBuild

        if [[ -e "$src/src/scanner.cc" ]]; then
          $CXX -c "$src/src/scanner.cc" -o scanner.o $FLAGS
        elif [[ -e "$src/src/scanner.c" ]]; then
          $CC -c "$src/src/scanner.c" -o scanner.o $FLAGS
        fi

        $CC -c "$src/src/parser.c" -o parser.o $FLAGS
        $CXX -shared -o $SHARED_LIB *.o

        runHook postBuild
      '';

      installPhase = ''
        runHook preInstall
        mkdir $out
        mv $SHARED_LIB $out/
        runHook postInstall
      '';

      # Strip failed on darwin: strip: error: symbols referenced by indirect symbol table entries that can't be stripped
      fixupPhase = lib.optionalString stdenv.isLinux ''
        runHook preFixup
        $STRIP $out/$SHARED_LIB
        runHook postFixup
      '';
    };
  grammarsToBuild = builtins.filter includeGrammarIf gitGrammars;
  builtGrammars =
    builtins.map (grammar: {
      inherit (grammar) name;
      value = buildGrammar grammar;
    })
    grammarsToBuild;
  extensibleGrammars =
    lib.makeExtensible (self: builtins.listToAttrs builtGrammars);
  overlaidGrammars =
    lib.pipe extensibleGrammars
    (builtins.map (overlay: grammar: grammar.extend overlay) grammarOverlays);
  sharedLibExtension = stdenv.hostPlatform.extensions.sharedLibrary;
  grammarLinks =
    lib.mapAttrsToList
    (name: artifact: "ln -s ${artifact}/${name}${sharedLibExtension} $out/${name}${sharedLibExtension}")
    (lib.filterAttrs (n: v: lib.isDerivation v) overlaidGrammars);
in
  runCommand "consolidated-helix-grammars" {} ''
    mkdir -p $out
    ${builtins.concatStringsSep "\n" grammarLinks}
  ''
