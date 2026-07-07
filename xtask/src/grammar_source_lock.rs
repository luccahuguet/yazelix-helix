use crate::path;
use crate::DynError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use toml::Value;

const LOCK_VERSION: u32 = 1;
const GITHUB_PREFIX: &str = "https://github.com/";
const CODEBERG_PREFIX: &str = "https://codeberg.org/";
const SOURCEHUT_PREFIX: &str = "https://git.sr.ht/";

#[derive(Debug, Clone)]
struct GrammarSource {
    name: String,
    git: String,
    rev: String,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct PrefetchKey {
    kind: PrefetchKind,
    identity: String,
    rev: String,
    sparse_checkout: Vec<String>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum PrefetchKind {
    Github,
    Archive,
    Git,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct GrammarLockEntry {
    fetcher: String,
    hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repo: Option<String>,
    rev: String,
    #[serde(rename = "sparseCheckout", skip_serializing_if = "Option::is_none")]
    sparse_checkout: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GrammarLockFile {
    grammars: BTreeMap<String, GrammarLockEntry>,
    version: u32,
}

fn lock_path() -> PathBuf {
    path::project_root().join("grammar_sources.lock.json")
}

fn languages_path() -> PathBuf {
    path::project_root().join("languages.toml")
}

fn parse_github(url: &str) -> Result<(String, String), DynError> {
    let rest = url
        .strip_prefix(GITHUB_PREFIX)
        .ok_or_else(|| format!("invalid GitHub grammar URL: {url}"))?;
    let mut parts = rest.split('/');
    let owner = parts
        .next()
        .ok_or_else(|| format!("invalid GitHub grammar URL: {url}"))?
        .to_string();
    let repo = parts
        .next()
        .ok_or_else(|| format!("invalid GitHub grammar URL: {url}"))?
        .trim_end_matches('/')
        .to_string();
    Ok((owner, repo))
}

fn archive_url(git: &str, rev: &str) -> Option<String> {
    let trimmed = git.trim_end_matches('/');
    if trimmed.starts_with(CODEBERG_PREFIX) {
        Some(format!("{trimmed}/archive/{rev}.tar.gz"))
    } else if trimmed.starts_with(SOURCEHUT_PREFIX) {
        Some(format!("{trimmed}/archive/{rev}.tar.gz"))
    } else {
        None
    }
}

fn load_languages_config() -> Result<Value, DynError> {
    let contents = fs::read_to_string(languages_path())?;
    Ok(toml::from_str(&contents)?)
}

fn active_grammar_sources(config: &Value) -> Result<Vec<GrammarSource>, DynError> {
    let use_grammars = config.get("use-grammars");
    let only = use_grammars
        .and_then(|value| value.get("only"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<HashSet<_>>()
        });
    let except = use_grammars
        .and_then(|value| value.get("except"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();

    let mut sources = Vec::new();
    let Some(grammars) = config.get("grammar").and_then(Value::as_array) else {
        return Ok(sources);
    };

    for grammar in grammars {
        let Some(name) = grammar.get("name").and_then(Value::as_str) else {
            continue;
        };
        if let Some(only) = &only {
            if !only.contains(name) {
                continue;
            }
        }
        if except.contains(name) {
            continue;
        }
        let Some(source) = grammar.get("source") else {
            continue;
        };
        let Some(git) = source.get("git").and_then(Value::as_str) else {
            continue;
        };
        let Some(rev) = source.get("rev").and_then(Value::as_str) else {
            continue;
        };
        sources.push(GrammarSource {
            name: name.to_string(),
            git: git.to_string(),
            rev: rev.to_string(),
        });
    }

    sources.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(sources)
}

impl GrammarSource {
    fn sparse_checkout(&self) -> Option<Vec<String>> {
        let src_only = matches!(
            (self.name.as_str(), self.git.as_str()),
            (
                "rpmspec",
                "https://gitlab.com/cryptomilk/tree-sitter-rpmspec"
            ) | (
                "wikitext",
                "https://github.com/santhoshtr/tree-sitter-wikitext"
            )
        );
        src_only.then(|| vec!["src".to_string()])
    }

    fn prefetch_key(&self) -> Result<PrefetchKey, DynError> {
        let sparse_checkout = self.sparse_checkout().unwrap_or_default();
        if self.git.starts_with(GITHUB_PREFIX) {
            let (owner, repo) = parse_github(&self.git)?;
            Ok(PrefetchKey {
                kind: PrefetchKind::Github,
                identity: format!("{owner}/{repo}"),
                rev: self.rev.clone(),
                sparse_checkout,
            })
        } else if let Some(url) = archive_url(&self.git, &self.rev) {
            Ok(PrefetchKey {
                kind: PrefetchKind::Archive,
                identity: url,
                rev: self.rev.clone(),
                sparse_checkout,
            })
        } else {
            Ok(PrefetchKey {
                kind: PrefetchKind::Git,
                identity: self.git.clone(),
                rev: self.rev.clone(),
                sparse_checkout,
            })
        }
    }

    fn lock_entry(&self, hash: &str) -> Result<GrammarLockEntry, DynError> {
        let sparse_checkout = self.sparse_checkout();
        if self.git.starts_with(GITHUB_PREFIX) {
            let (owner, repo) = parse_github(&self.git)?;
            Ok(GrammarLockEntry {
                fetcher: "github".to_string(),
                hash: hash.to_string(),
                owner: Some(owner),
                repo: Some(repo),
                rev: self.rev.clone(),
                sparse_checkout,
                url: None,
            })
        } else if archive_url(&self.git, &self.rev).is_some() {
            Ok(GrammarLockEntry {
                fetcher: "archive".to_string(),
                hash: hash.to_string(),
                owner: None,
                repo: None,
                rev: self.rev.clone(),
                sparse_checkout,
                url: Some(self.git.clone()),
            })
        } else {
            Ok(GrammarLockEntry {
                fetcher: "git".to_string(),
                hash: hash.to_string(),
                owner: None,
                repo: None,
                rev: self.rev.clone(),
                sparse_checkout,
                url: Some(self.git.clone()),
            })
        }
    }
}

fn run_nix_json(
    command: &str,
    package: &str,
    args: &[String],
) -> Result<serde_json::Value, DynError> {
    let output = Command::new("nix")
        .arg("run")
        .arg(format!("nixpkgs#{package}"))
        .arg("--")
        .arg(command)
        .args(args)
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "nix prefetch failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    Ok(serde_json::from_slice(&output.stdout)?)
}

fn prefetch_github(owner: &str, repo: &str, rev: &str) -> Result<String, DynError> {
    let payload = run_nix_json(
        "--json",
        "nix-prefetch-github",
        &[
            "--rev".to_string(),
            rev.to_string(),
            owner.to_string(),
            repo.to_string(),
        ],
    )?;
    payload
        .get("hash")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "nix-prefetch-github output missing hash".into())
}

fn prefetch_git(url: &str, rev: &str, sparse_checkout: &[String]) -> Result<String, DynError> {
    let mut args = vec![
        "--url".to_string(),
        url.to_string(),
        "--rev".to_string(),
        rev.to_string(),
    ];
    for path in sparse_checkout {
        args.push("--sparse-checkout".to_string());
        args.push(path.clone());
    }
    let payload = run_nix_json("--json", "nix-prefetch-git", &args)?;
    payload
        .get("hash")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "nix-prefetch-git output missing hash".into())
}

fn prefetch_archive(git: &str, rev: &str) -> Result<String, DynError> {
    let url = archive_url(git, rev)
        .ok_or_else(|| format!("no archive fetcher is defined for grammar URL: {git}"))?;
    let output = Command::new("nix")
        .args(["store", "prefetch-file", "--json", "--unpack", &url])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "nix archive prefetch failed for {url}: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    let payload: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    payload
        .get("hash")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "nix store prefetch-file output missing hash".into())
}

fn prefetch_source(source: &GrammarSource) -> Result<String, DynError> {
    let sparse_checkout = source.sparse_checkout().unwrap_or_default();
    if !sparse_checkout.is_empty() {
        prefetch_git(&source.git, &source.rev, &sparse_checkout)
    } else if source.git.starts_with(GITHUB_PREFIX) {
        let (owner, repo) = parse_github(&source.git)?;
        prefetch_github(&owner, &repo, &source.rev)
    } else if archive_url(&source.git, &source.rev).is_some() {
        prefetch_archive(&source.git, &source.rev)
    } else {
        prefetch_git(&source.git, &source.rev, &sparse_checkout)
    }
}

fn build_lock(sources: &[GrammarSource], jobs: usize) -> Result<GrammarLockFile, DynError> {
    use std::sync::{Arc, Mutex};

    let mut unique_sources: HashMap<PrefetchKey, GrammarSource> = HashMap::new();
    for source in sources {
        unique_sources
            .entry(source.prefetch_key()?)
            .or_insert_with(|| source.clone());
    }

    let hashes = Arc::new(Mutex::new(HashMap::<PrefetchKey, String>::new()));
    let failures = Arc::new(Mutex::new(Vec::<String>::new()));
    let jobs = jobs.max(1);

    for chunk in unique_sources.into_iter().collect::<Vec<_>>().chunks(jobs) {
        let handles = chunk
            .iter()
            .map(|(key, source)| {
                let key = key.clone();
                let source = source.clone();
                let hashes = Arc::clone(&hashes);
                let failures = Arc::clone(&failures);
                thread::spawn(move || match prefetch_source(&source) {
                    Ok(hash) => {
                        hashes
                            .lock()
                            .expect("hash mutex poisoned")
                            .insert(key, hash);
                    }
                    Err(error) => failures
                        .lock()
                        .expect("failure mutex poisoned")
                        .push(error.to_string()),
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            handle.join().expect("prefetch thread panicked");
        }
    }

    let failures = failures.lock().expect("failure mutex poisoned").clone();
    if !failures.is_empty() {
        return Err(format!("grammar source prefetch failed:\n{}", failures.join("\n")).into());
    }

    let hashes = hashes.lock().expect("hash mutex poisoned").clone();
    let mut grammars = BTreeMap::new();
    for source in sources {
        let key = source.prefetch_key()?;
        let hash = hashes
            .get(&key)
            .ok_or_else(|| format!("missing prefetch hash for grammar '{}'", source.name))?;
        grammars.insert(source.name.clone(), source.lock_entry(hash)?);
    }

    Ok(GrammarLockFile {
        grammars,
        version: LOCK_VERSION,
    })
}

fn load_lock() -> Result<GrammarLockFile, DynError> {
    let contents = fs::read_to_string(lock_path())?;
    Ok(serde_json::from_str(&contents)?)
}

fn write_lock(lock: &GrammarLockFile) -> Result<(), DynError> {
    let contents = serde_json::to_string_pretty(lock)? + "\n";
    fs::write(lock_path(), contents)?;
    Ok(())
}

pub fn validate() -> Result<(), DynError> {
    let errors = collect_validation_errors()?;
    if errors.is_empty() {
        println!("grammar source lock is valid ({})", lock_path().display());
        return Ok(());
    }

    eprintln!("grammar source lock validation failed:");
    for error in errors {
        eprintln!("- {error}");
    }
    Err("grammar source lock validation failed".into())
}

fn collect_validation_errors() -> Result<Vec<String>, DynError> {
    let config = load_languages_config()?;
    let sources = active_grammar_sources(&config)?;
    if !lock_path().exists() {
        return Ok(vec![format!(
            "missing lock file: {}",
            lock_path().display()
        )]);
    }

    let lock = load_lock()?;
    let mut errors = Vec::new();

    if lock.version != LOCK_VERSION {
        errors.push(format!(
            "unsupported lock version {}; expected {LOCK_VERSION}",
            lock.version
        ));
    }

    let expected_names: HashSet<_> = sources.iter().map(|source| source.name.as_str()).collect();
    let actual_names: HashSet<_> = lock.grammars.keys().map(String::as_str).collect();

    let missing = expected_names
        .difference(&actual_names)
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        errors.push(format!("missing lock entries: {}", missing.join(", ")));
    }

    let extra = actual_names
        .difference(&expected_names)
        .copied()
        .collect::<Vec<_>>();
    if !extra.is_empty() {
        errors.push(format!("stale lock entries: {}", extra.join(", ")));
    }

    for source in &sources {
        let Some(entry) = lock.grammars.get(&source.name) else {
            continue;
        };
        if entry.rev != source.rev {
            errors.push(format!(
                "{}: lock rev {} != languages.toml rev {}",
                source.name, entry.rev, source.rev
            ));
        }
        if source.git.starts_with(GITHUB_PREFIX) {
            let (owner, repo) = parse_github(&source.git)?;
            if entry.fetcher != "github" {
                errors.push(format!("{}: expected github fetcher", source.name));
            }
            if entry.owner.as_deref() != Some(owner.as_str())
                || entry.repo.as_deref() != Some(repo.as_str())
            {
                errors.push(format!("{}: github owner/repo drift", source.name));
            }
        } else if archive_url(&source.git, &source.rev).is_some() {
            if entry.fetcher != "archive" {
                errors.push(format!(
                    "{}: expected archive fetcher for archive-capable URL",
                    source.name
                ));
            }
            if entry.url.as_deref() != Some(source.git.as_str()) {
                errors.push(format!("{}: archive url drift", source.name));
            }
        } else if entry.fetcher != "git" {
            errors.push(format!("{}: expected git fetcher", source.name));
        } else if entry.url.as_deref() != Some(source.git.as_str()) {
            errors.push(format!("{}: git url drift", source.name));
        }
    }

    Ok(errors)
}

pub fn update(selected: Vec<String>, jobs: usize) -> Result<(), DynError> {
    let config = load_languages_config()?;
    let mut sources = active_grammar_sources(&config)?;
    let partial_update = !selected.is_empty();

    if partial_update {
        let selected: HashSet<_> = selected.into_iter().collect();
        sources.retain(|source| selected.contains(&source.name));
        if sources.is_empty() {
            return Err("no matching grammars selected".into());
        }
    }

    println!(
        "prefetching {} grammar sources with {jobs} workers",
        sources.len()
    );

    let refreshed = build_lock(&sources, jobs)?;
    let lock = if partial_update && lock_path().exists() {
        let mut existing = load_lock()?;
        existing.grammars.extend(refreshed.grammars);
        existing.version = LOCK_VERSION;
        existing
    } else {
        refreshed
    };

    let entry_count = lock.grammars.len();
    write_lock(&lock)?;
    println!("wrote {} ({entry_count} entries)", lock_path().display());
    Ok(())
}
