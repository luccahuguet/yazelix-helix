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
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum PrefetchKind {
    Github,
    Git,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct GrammarLockEntry {
    fetcher: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    rev: String,
    hash: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GrammarLockFile {
    version: u32,
    grammars: BTreeMap<String, GrammarLockEntry>,
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
    fn prefetch_key(&self) -> Result<PrefetchKey, DynError> {
        if self.git.starts_with(GITHUB_PREFIX) {
            let (owner, repo) = parse_github(&self.git)?;
            Ok(PrefetchKey {
                kind: PrefetchKind::Github,
                identity: format!("{owner}/{repo}"),
                rev: self.rev.clone(),
            })
        } else {
            Ok(PrefetchKey {
                kind: PrefetchKind::Git,
                identity: self.git.clone(),
                rev: self.rev.clone(),
            })
        }
    }

    fn lock_entry(&self, hash: &str) -> Result<GrammarLockEntry, DynError> {
        if self.git.starts_with(GITHUB_PREFIX) {
            let (owner, repo) = parse_github(&self.git)?;
            Ok(GrammarLockEntry {
                fetcher: "github".to_string(),
                owner: Some(owner),
                repo: Some(repo),
                url: None,
                rev: self.rev.clone(),
                hash: hash.to_string(),
            })
        } else {
            Ok(GrammarLockEntry {
                fetcher: "git".to_string(),
                owner: None,
                repo: None,
                url: Some(self.git.clone()),
                rev: self.rev.clone(),
                hash: hash.to_string(),
            })
        }
    }
}

fn run_nix_json(command: &str, package: &str, args: &[&str]) -> Result<serde_json::Value, DynError> {
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
        &["--rev", rev, owner, repo],
    )?;
    payload
        .get("hash")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "nix-prefetch-github output missing hash".into())
}

fn prefetch_git(url: &str, rev: &str) -> Result<String, DynError> {
    let payload = run_nix_json(
        "--json",
        "nix-prefetch-git",
        &["--url", url, "--rev", rev],
    )?;
    payload
        .get("hash")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "nix-prefetch-git output missing hash".into())
}

fn prefetch_source(source: &GrammarSource) -> Result<String, DynError> {
    if source.git.starts_with(GITHUB_PREFIX) {
        let (owner, repo) = parse_github(&source.git)?;
        prefetch_github(&owner, &repo, &source.rev)
    } else {
        prefetch_git(&source.git, &source.rev)
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

    for chunk in unique_sources
        .into_iter()
        .collect::<Vec<_>>()
        .chunks(jobs)
    {
        let handles = chunk
            .iter()
            .map(|(key, source)| {
                let key = key.clone();
                let source = source.clone();
                let hashes = Arc::clone(&hashes);
                let failures = Arc::clone(&failures);
                thread::spawn(move || {
                    match prefetch_source(&source) {
                        Ok(hash) => {
                            hashes.lock().expect("hash mutex poisoned").insert(key, hash);
                        }
                        Err(error) => failures
                            .lock()
                            .expect("failure mutex poisoned")
                            .push(error.to_string()),
                    }
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
        version: LOCK_VERSION,
        grammars,
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
        return Ok(vec![format!("missing lock file: {}", lock_path().display())]);
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