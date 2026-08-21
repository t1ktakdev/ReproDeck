use crate::git_ignore::GitIgnoreMatcher;
use crate::redaction::{self, RedactionResult};
use git2::{Repository, Status, StatusOptions};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, SystemTimeError, UNIX_EPOCH};
use thiserror::Error;
use walkdir::{DirEntry, WalkDir};

const PROFILE_SCHEMA_VERSION: u32 = 1;
const MAX_FILES_SCANNED: usize = 40_000;
const MAX_TEXT_BYTES: u64 = 512 * 1024;
const MAX_README_BYTES: u64 = 64 * 1024;

#[derive(Debug, Error)]
pub enum ProjectIntelligenceError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Clock(#[from] SystemTimeError),
    #[error("project directory does not exist: {0}")]
    MissingDirectory(String),
    #[error("project path is not a directory: {0}")]
    NotDirectory(String),
    #[error("project path is not valid UTF-8")]
    NonUtf8Path,
}

pub type Result<T> = std::result::Result<T, ProjectIntelligenceError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectProfile {
    pub schema_version: u32,
    pub fingerprint: String,
    pub root_path: String,
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub analyzed_at: i64,
    pub git: Option<ProjectGitState>,
    pub languages: Vec<LanguageStat>,
    pub technologies: Vec<TechnologySignal>,
    pub commands: Vec<ProjectCommand>,
    pub entrypoints: Vec<String>,
    pub test_paths: Vec<String>,
    pub documentation: Vec<String>,
    pub ci_files: Vec<String>,
    pub signals: Vec<ProjectSignal>,
    pub stats: ProjectStats,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectGitState {
    pub root_path: String,
    pub branch: String,
    pub head_commit: Option<String>,
    pub is_dirty: bool,
    pub changed_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LanguageStat {
    pub language: String,
    pub files: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TechnologySignal {
    pub name: String,
    pub category: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectCommand {
    pub id: String,
    pub label: String,
    pub kind: ProjectCommandKind,
    pub executable: String,
    pub args: Vec<String>,
    pub source: String,
    pub confidence: CommandConfidence,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProjectCommandKind {
    Build,
    Test,
    Lint,
    Typecheck,
    Dev,
    Check,
    Other,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CommandConfidence {
    Declared,
    Conventional,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProjectSignalSeverity {
    Info,
    Review,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectSignal {
    pub id: String,
    pub severity: ProjectSignalSeverity,
    pub title: String,
    pub detail: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ProjectStats {
    pub files_seen: usize,
    pub source_files: usize,
    pub test_files: usize,
    pub documentation_files: usize,
    pub sensitive_files_excluded: usize,
    pub skipped_large_files: usize,
    pub todo_markers: usize,
    pub scan_truncated: bool,
}

#[derive(Debug, Clone)]
struct ScanFacts {
    root: PathBuf,
    relative_files: Vec<String>,
    extensions: BTreeMap<String, usize>,
    source_files: usize,
    test_paths: Vec<String>,
    test_file_count: usize,
    documentation: Vec<String>,
    documentation_file_count: usize,
    ci_files: Vec<String>,
    entrypoints: BTreeSet<String>,
    sensitive_files_excluded: usize,
    skipped_large_files: usize,
    todo_markers: usize,
    scan_truncated: bool,
}

fn unix_time_secs() -> Result<i64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64)
}

fn should_descend(entry: &DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return true;
    }
    let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
    !matches!(
        name.as_str(),
        ".git"
            | "node_modules"
            | "target"
            | "dist"
            | "build"
            | ".next"
            | "coverage"
            | "vendor"
            | ".venv"
            | "venv"
            | "__pycache__"
            | ".idea"
            | ".gradle"
            | ".cache"
            | ".turbo"
    )
}

fn relative_text(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    if relative.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return None;
    }
    Some(relative.to_string_lossy().replace('\\', "/"))
}

fn is_sensitive_relative(path: &Path) -> bool {
    let segments = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if segments
        .iter()
        .any(|segment| matches!(segment.as_str(), ".ssh" | ".aws" | ".gnupg" | ".azure"))
    {
        return true;
    }
    !matches!(redaction::redact_path(path), RedactionResult::Included(_))
}

fn is_text_candidate(path: &Path) -> bool {
    let Some(extension) = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
    else {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        return matches!(
            name.as_str(),
            "dockerfile" | "makefile" | "justfile" | "procfile"
        );
    };
    matches!(
        extension.as_str(),
        "rs" | "toml"
            | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "mjs"
            | "cjs"
            | "json"
            | "md"
            | "mdx"
            | "py"
            | "go"
            | "java"
            | "kt"
            | "kts"
            | "cs"
            | "fs"
            | "fsx"
            | "php"
            | "rb"
            | "swift"
            | "c"
            | "h"
            | "cc"
            | "cpp"
            | "hpp"
            | "vue"
            | "svelte"
            | "html"
            | "css"
            | "scss"
            | "sass"
            | "less"
            | "yaml"
            | "yml"
            | "xml"
            | "gradle"
            | "properties"
            | "sh"
            | "ps1"
            | "cmd"
            | "bat"
            | "sql"
    )
}

fn language_for_extension(extension: &str) -> Option<&'static str> {
    match extension {
        "rs" => Some("Rust"),
        "ts" | "tsx" => Some("TypeScript"),
        "js" | "jsx" | "mjs" | "cjs" => Some("JavaScript"),
        "py" => Some("Python"),
        "go" => Some("Go"),
        "java" => Some("Java"),
        "kt" | "kts" => Some("Kotlin"),
        "cs" => Some("C#"),
        "fs" | "fsx" => Some("F#"),
        "php" => Some("PHP"),
        "rb" => Some("Ruby"),
        "swift" => Some("Swift"),
        "c" | "h" => Some("C"),
        "cc" | "cpp" | "hpp" => Some("C++"),
        "vue" => Some("Vue"),
        "svelte" => Some("Svelte"),
        "html" => Some("HTML"),
        "css" | "scss" | "sass" | "less" => Some("CSS"),
        "sh" => Some("Shell"),
        "ps1" => Some("PowerShell"),
        "sql" => Some("SQL"),
        _ => None,
    }
}

fn looks_like_test(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("/tests/")
        || lower.contains("/test/")
        || lower.contains("/__tests__/")
        || lower.ends_with("_test.go")
        || lower.ends_with("_test.rs")
        || lower.contains(".test.")
        || lower.contains(".spec.")
        || lower.starts_with("tests/")
        || lower.starts_with("test/")
}

fn is_documentation(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower == "readme.md"
        || lower == "readme"
        || lower == "contributing.md"
        || lower == "security.md"
        || lower == "changelog.md"
        || lower.starts_with("docs/")
}

fn is_ci(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.starts_with(".github/workflows/")
        || lower == ".gitlab-ci.yml"
        || lower == "azure-pipelines.yml"
        || lower == "jenkinsfile"
        || lower.starts_with(".circleci/")
}

fn detect_entrypoint(path: &str) -> bool {
    matches!(
        path.to_ascii_lowercase().as_str(),
        "src/main.rs"
            | "src/lib.rs"
            | "src/main.ts"
            | "src/main.tsx"
            | "src/index.ts"
            | "src/index.tsx"
            | "src/main.js"
            | "src/index.js"
            | "main.py"
            | "app.py"
            | "manage.py"
            | "cmd/main.go"
            | "main.go"
            | "program.cs"
    )
}

fn count_markers(text: &str) -> usize {
    text.match_indices("TODO").count()
        + text.match_indices("FIXME").count()
        + text.match_indices("HACK").count()
}

fn scan_project(root: &Path) -> Result<ScanFacts> {
    let mut relative_files = Vec::new();
    let mut extensions = BTreeMap::new();
    let mut source_files = 0usize;
    let mut test_paths = Vec::new();
    let mut test_file_count = 0usize;
    let mut documentation = Vec::new();
    let mut documentation_file_count = 0usize;
    let mut ci_files = Vec::new();
    let mut entrypoints = BTreeSet::new();
    let mut sensitive_files_excluded = 0usize;
    let mut skipped_large_files = 0usize;
    let mut todo_markers = 0usize;
    let mut scan_truncated = false;
    let mut file_entries_seen = 0usize;

    let git_ignore = GitIgnoreMatcher::discover(root);
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| should_descend(entry) && !git_ignore.is_ignored(entry.path()))
    {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            continue;
        }
        file_entries_seen = file_entries_seen.saturating_add(1);
        if file_entries_seen > MAX_FILES_SCANNED {
            scan_truncated = true;
            break;
        }
        let Some(relative) = relative_text(root, entry.path()) else {
            continue;
        };
        let relative_path = Path::new(&relative);
        if is_sensitive_relative(relative_path) {
            sensitive_files_excluded += 1;
            continue;
        }
        relative_files.push(relative.clone());
        if let Some(extension) = relative_path
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
        {
            *extensions.entry(extension).or_insert(0) += 1;
        }
        if detect_entrypoint(&relative) {
            entrypoints.insert(relative.clone());
        }
        if looks_like_test(&relative) {
            test_file_count += 1;
            test_paths.push(relative.clone());
        }
        if is_documentation(&relative) {
            documentation_file_count += 1;
            documentation.push(relative.clone());
        }
        if is_ci(&relative) {
            ci_files.push(relative.clone());
        }
        if is_text_candidate(relative_path) {
            source_files += 1;
            let size = entry
                .metadata()
                .map(|metadata| metadata.len())
                .unwrap_or(MAX_TEXT_BYTES + 1);
            if size > MAX_TEXT_BYTES {
                skipped_large_files += 1;
                continue;
            }
            if let Ok(bytes) = fs::read(entry.path()) {
                if bytes.contains(&0) {
                    continue;
                }
                let text = String::from_utf8_lossy(&bytes);
                todo_markers = todo_markers.saturating_add(count_markers(&text));
            }
        }
    }

    test_paths.sort();
    test_paths.truncate(100);
    documentation.sort();
    documentation.truncate(100);
    ci_files.sort();
    ci_files.truncate(100);

    Ok(ScanFacts {
        root: root.to_path_buf(),
        relative_files,
        extensions,
        source_files,
        test_paths,
        test_file_count,
        documentation,
        documentation_file_count,
        ci_files,
        entrypoints,
        sensitive_files_excluded,
        skipped_large_files,
        todo_markers,
        scan_truncated,
    })
}

fn read_capped(path: &Path, max_bytes: u64) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    let text = read_capped(path, MAX_TEXT_BYTES)?;
    serde_json::from_str(&text).ok()
}

fn read_readme_description(root: &Path) -> Option<String> {
    for name in ["README.md", "README.MD", "README", "readme.md"] {
        let Some(text) = read_capped(&root.join(name), MAX_README_BYTES) else {
            continue;
        };
        for block in text.split("\n\n") {
            let cleaned = block
                .lines()
                .map(str::trim)
                .filter(|line| {
                    !line.is_empty()
                        && !line.starts_with('#')
                        && !line.starts_with("![")
                        && !line.starts_with("[![")
                })
                .collect::<Vec<_>>()
                .join(" ");
            if cleaned.len() >= 20 {
                return Some(cleaned.chars().take(400).collect());
            }
        }
    }
    None
}

fn inspect_git(root: &Path) -> Option<ProjectGitState> {
    let repository = Repository::discover(root).ok()?;
    let workdir = repository.workdir()?.canonicalize().ok()?;
    let head = repository.head().ok();
    let branch = head
        .as_ref()
        .and_then(|value| value.shorthand())
        .unwrap_or("HEAD")
        .to_string();
    let head_commit = head
        .as_ref()
        .and_then(|value| value.target())
        .map(|oid| oid.to_string());
    drop(head);
    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false);
    let mut changed_files = Vec::new();
    if let Ok(statuses) = repository.statuses(Some(&mut options)) {
        for entry in statuses.iter() {
            if entry.status() == Status::CURRENT {
                continue;
            }
            if let Some(path) = entry.path() {
                changed_files.push(path.replace('\\', "/"));
            }
        }
    }
    changed_files.sort();
    changed_files.dedup();
    Some(ProjectGitState {
        root_path: workdir.to_string_lossy().into_owned(),
        branch,
        head_commit,
        is_dirty: !changed_files.is_empty(),
        changed_files,
    })
}

fn node_package_manager(facts: &ScanFacts) -> &'static str {
    if facts
        .relative_files
        .iter()
        .any(|path| path == "pnpm-lock.yaml")
    {
        "pnpm"
    } else if facts.relative_files.iter().any(|path| path == "yarn.lock") {
        "yarn"
    } else {
        "npm"
    }
}

fn command_for_script(manager: &str, script: &str) -> (String, Vec<String>) {
    match manager {
        "yarn" => ("yarn".into(), vec![script.into()]),
        "pnpm" => ("pnpm".into(), vec!["run".into(), script.into()]),
        _ => {
            if script == "test" {
                ("npm".into(), vec!["test".into()])
            } else {
                ("npm".into(), vec!["run".into(), script.into()])
            }
        }
    }
}

fn classify_script(name: &str) -> ProjectCommandKind {
    let lower = name.to_ascii_lowercase();
    if lower == "test" || lower.starts_with("test:") {
        ProjectCommandKind::Test
    } else if lower == "build" || lower.starts_with("build:") {
        ProjectCommandKind::Build
    } else if lower == "lint" || lower.starts_with("lint:") {
        ProjectCommandKind::Lint
    } else if lower.contains("typecheck") || lower.contains("type-check") || lower == "check:types"
    {
        ProjectCommandKind::Typecheck
    } else if matches!(lower.as_str(), "dev" | "start" | "serve") || lower.starts_with("dev:") {
        ProjectCommandKind::Dev
    } else if lower == "check" || lower.starts_with("check:") {
        ProjectCommandKind::Check
    } else {
        ProjectCommandKind::Other
    }
}

fn push_command(
    commands: &mut Vec<ProjectCommand>,
    label: impl Into<String>,
    kind: ProjectCommandKind,
    executable: impl Into<String>,
    args: Vec<String>,
    source: impl Into<String>,
    confidence: CommandConfidence,
) {
    let label = label.into();
    let executable = executable.into();
    let source = source.into();
    let mut hasher = Sha256::new();
    hasher.update(executable.as_bytes());
    for arg in &args {
        hasher.update([0]);
        hasher.update(arg.as_bytes());
    }
    hasher.update(source.as_bytes());
    let id = format!("cmd:{}", &hex::encode(hasher.finalize())[..12]);
    if commands.iter().any(|command| command.id == id) {
        return;
    }
    commands.push(ProjectCommand {
        id,
        label,
        kind,
        executable,
        args,
        source,
        confidence,
    });
}

#[derive(Debug, Default)]
struct PackageMetadata {
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
    commands: Vec<ProjectCommand>,
    dependencies: BTreeSet<String>,
}

fn package_metadata(facts: &ScanFacts) -> PackageMetadata {
    let path = facts.root.join("package.json");
    let Some(value) = read_json(&path) else {
        return PackageMetadata::default();
    };
    let name = value
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let version = value
        .get("version")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let description = value
        .get("description")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string);
    let manager = node_package_manager(facts);
    let mut commands = Vec::new();
    if let Some(scripts) = value.get("scripts").and_then(|value| value.as_object()) {
        for script in [
            "dev",
            "start",
            "build",
            "typecheck",
            "type-check",
            "lint",
            "test",
            "check",
        ] {
            if scripts.contains_key(script) {
                let (executable, args) = command_for_script(manager, script);
                push_command(
                    &mut commands,
                    script,
                    classify_script(script),
                    executable,
                    args,
                    format!("package.json scripts.{script}"),
                    CommandConfidence::Declared,
                );
            }
        }
        for script in scripts
            .keys()
            .filter(|script| {
                script.starts_with("test:")
                    || script.starts_with("lint:")
                    || script.starts_with("check:")
            })
            .take(12)
        {
            let (executable, args) = command_for_script(manager, script);
            push_command(
                &mut commands,
                script,
                classify_script(script),
                executable,
                args,
                format!("package.json scripts.{script}"),
                CommandConfidence::Declared,
            );
        }
    }
    let mut dependencies = BTreeSet::new();
    for section in ["dependencies", "devDependencies", "peerDependencies"] {
        if let Some(values) = value.get(section).and_then(|value| value.as_object()) {
            dependencies.extend(values.keys().cloned());
        }
    }
    PackageMetadata {
        name,
        version,
        description,
        commands,
        dependencies,
    }
}

fn add_technology(
    technologies: &mut Vec<TechnologySignal>,
    name: &str,
    category: &str,
    evidence: impl Into<String>,
) {
    let evidence = evidence.into();
    if let Some(existing) = technologies.iter_mut().find(|item| item.name == name) {
        if !existing.evidence.contains(&evidence) {
            existing.evidence.push(evidence);
        }
        return;
    }
    technologies.push(TechnologySignal {
        name: name.into(),
        category: category.into(),
        evidence: vec![evidence],
    });
}

fn detect_technologies(
    facts: &ScanFacts,
    dependencies: &BTreeSet<String>,
    commands: &mut Vec<ProjectCommand>,
) -> Vec<TechnologySignal> {
    let mut technologies = Vec::new();
    let has = |path: &str| {
        facts
            .relative_files
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(path))
    };

    if has("package.json") {
        add_technology(&mut technologies, "Node.js", "runtime", "package.json");
        add_technology(
            &mut technologies,
            node_package_manager(facts),
            "package-manager",
            if has("pnpm-lock.yaml") {
                "pnpm-lock.yaml"
            } else if has("yarn.lock") {
                "yarn.lock"
            } else {
                "package.json"
            },
        );
    }
    for (dependency, name, category) in [
        ("react", "React", "framework"),
        ("next", "Next.js", "framework"),
        ("vue", "Vue", "framework"),
        ("svelte", "Svelte", "framework"),
        ("@tauri-apps/api", "Tauri", "desktop"),
        ("electron", "Electron", "desktop"),
        ("vite", "Vite", "build-tool"),
        ("typescript", "TypeScript", "language-tooling"),
        ("vitest", "Vitest", "test"),
        ("jest", "Jest", "test"),
        ("playwright", "Playwright", "test"),
        ("@playwright/test", "Playwright", "test"),
    ] {
        if dependencies.contains(dependency) {
            add_technology(
                &mut technologies,
                name,
                category,
                format!("package.json dependency {dependency}"),
            );
        }
    }
    if has("Cargo.toml") {
        add_technology(&mut technologies, "Cargo", "build-tool", "Cargo.toml");
        add_technology(&mut technologies, "Rust", "language", "Cargo.toml");
        push_command(
            commands,
            "cargo check",
            ProjectCommandKind::Check,
            "cargo",
            vec!["check".into(), "--workspace".into()],
            "Cargo.toml",
            CommandConfidence::Conventional,
        );
        push_command(
            commands,
            "cargo test",
            ProjectCommandKind::Test,
            "cargo",
            vec!["test".into(), "--workspace".into()],
            "Cargo.toml",
            CommandConfidence::Conventional,
        );
        push_command(
            commands,
            "cargo clippy",
            ProjectCommandKind::Lint,
            "cargo",
            vec![
                "clippy".into(),
                "--workspace".into(),
                "--all-targets".into(),
                "--".into(),
                "-D".into(),
                "warnings".into(),
            ],
            "Cargo.toml",
            CommandConfidence::Conventional,
        );
        if let Some(cargo) = read_capped(&facts.root.join("Cargo.toml"), MAX_TEXT_BYTES) {
            if cargo.contains("tauri") {
                add_technology(
                    &mut technologies,
                    "Tauri",
                    "desktop",
                    "Cargo.toml contains tauri",
                );
            }
            if cargo.contains("tokio") {
                add_technology(
                    &mut technologies,
                    "Tokio",
                    "runtime",
                    "Cargo.toml contains tokio",
                );
            }
        }
    }
    if has("pyproject.toml") || has("requirements.txt") || has("setup.py") {
        add_technology(
            &mut technologies,
            "Python",
            "language",
            if has("pyproject.toml") {
                "pyproject.toml"
            } else {
                "requirements.txt/setup.py"
            },
        );
        if facts.test_paths.iter().any(|path| path.ends_with(".py")) {
            push_command(
                commands,
                "pytest",
                ProjectCommandKind::Test,
                "python",
                vec!["-m".into(), "pytest".into()],
                "Python test files",
                CommandConfidence::Conventional,
            );
        }
    }
    if has("go.mod") {
        add_technology(&mut technologies, "Go", "language", "go.mod");
        push_command(
            commands,
            "go test",
            ProjectCommandKind::Test,
            "go",
            vec!["test".into(), "./...".into()],
            "go.mod",
            CommandConfidence::Conventional,
        );
    }
    if has("pom.xml") {
        add_technology(&mut technologies, "Maven", "build-tool", "pom.xml");
        push_command(
            commands,
            "mvn test",
            ProjectCommandKind::Test,
            "mvn",
            vec!["test".into()],
            "pom.xml",
            CommandConfidence::Conventional,
        );
    }
    if has("build.gradle") || has("build.gradle.kts") {
        add_technology(
            &mut technologies,
            "Gradle",
            "build-tool",
            if has("build.gradle.kts") {
                "build.gradle.kts"
            } else {
                "build.gradle"
            },
        );
        let executable = if cfg!(windows) {
            "gradlew.bat"
        } else {
            "./gradlew"
        };
        if has("gradlew") || has("gradlew.bat") {
            push_command(
                commands,
                "Gradle test",
                ProjectCommandKind::Test,
                executable,
                vec!["test".into()],
                "Gradle wrapper",
                CommandConfidence::Conventional,
            );
        }
    }
    if facts
        .relative_files
        .iter()
        .any(|path| path.ends_with(".sln") || path.ends_with(".csproj"))
    {
        add_technology(&mut technologies, ".NET", "runtime", ".sln/.csproj");
        push_command(
            commands,
            "dotnet test",
            ProjectCommandKind::Test,
            "dotnet",
            vec!["test".into()],
            ".NET project",
            CommandConfidence::Conventional,
        );
    }
    if has("composer.json") {
        add_technology(
            &mut technologies,
            "Composer",
            "package-manager",
            "composer.json",
        );
    }
    if has("Gemfile") {
        add_technology(&mut technologies, "Bundler", "package-manager", "Gemfile");
    }
    technologies.sort_by(|a, b| {
        a.category
            .cmp(&b.category)
            .then_with(|| a.name.cmp(&b.name))
    });
    technologies
}

fn build_languages(facts: &ScanFacts) -> Vec<LanguageStat> {
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for (extension, count) in &facts.extensions {
        if let Some(language) = language_for_extension(extension) {
            *counts.entry(language).or_insert(0) += count;
        }
    }
    let mut languages = counts
        .into_iter()
        .map(|(language, files)| LanguageStat {
            language: language.into(),
            files,
        })
        .collect::<Vec<_>>();
    languages.sort_by(|a, b| {
        b.files
            .cmp(&a.files)
            .then_with(|| a.language.cmp(&b.language))
    });
    languages
}

fn build_signals(
    facts: &ScanFacts,
    git: Option<&ProjectGitState>,
    commands: &[ProjectCommand],
) -> Vec<ProjectSignal> {
    let mut signals = Vec::new();
    let mut push = |id: &str,
                    severity: ProjectSignalSeverity,
                    title: &str,
                    detail: String,
                    evidence: Vec<String>| {
        signals.push(ProjectSignal {
            id: id.into(),
            severity,
            title: title.into(),
            detail,
            evidence,
        });
    };

    if let Some(git) = git.filter(|git| git.is_dirty) {
        push(
            "git-dirty",
            ProjectSignalSeverity::Review,
            "Repository has local changes",
            format!(
                "{} changed path(s) are present before ReproDeck starts an investigation.",
                git.changed_files.len()
            ),
            git.changed_files.iter().take(12).cloned().collect(),
        );
    }
    if facts.test_paths.is_empty()
        && !commands
            .iter()
            .any(|command| command.kind == ProjectCommandKind::Test)
    {
        push("no-tests-detected", ProjectSignalSeverity::Warning, "No deterministic test surface detected", "ReproDeck could not find test files or a declared/conventional test command. Automated verification may need a user-provided check.".into(), Vec::new());
    }
    if facts.ci_files.is_empty() {
        push("no-ci-detected", ProjectSignalSeverity::Info, "No CI configuration detected", "This is not a bug, but there is no repository-local CI file ReproDeck can use as an additional source of verification commands.".into(), Vec::new());
    }
    if facts.todo_markers > 0 {
        push("maintenance-markers", ProjectSignalSeverity::Info, "Maintenance markers found", format!("Found {} TODO/FIXME/HACK marker(s) in bounded text scanning. These are review leads, not confirmed bugs.", facts.todo_markers), Vec::new());
    }
    if facts.scan_truncated {
        push("scan-truncated", ProjectSignalSeverity::Review, "Project scan reached its safety limit", format!("The deterministic discovery pass stopped after {MAX_FILES_SCANNED} files. ReproDeck will use targeted retrieval for deeper investigation instead of scanning everything eagerly."), Vec::new());
    }
    if facts.sensitive_files_excluded > 0 {
        push("sensitive-excluded", ProjectSignalSeverity::Info, "Sensitive paths excluded from analysis", format!("{} path(s) matched local secret/privacy rules and were not read by Project Intelligence.", facts.sensitive_files_excluded), Vec::new());
    }
    signals
}

fn project_fingerprint(root: &str, git: Option<&ProjectGitState>, files: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(root.as_bytes());
    if let Some(git) = git {
        hasher.update(git.head_commit.as_deref().unwrap_or("unborn").as_bytes());
    }
    for path in files.iter().take(2_000) {
        hasher.update([0]);
        hasher.update(path.as_bytes());
    }
    format!("project:{}", &hex::encode(hasher.finalize())[..20])
}

pub fn analyze_project(path: &Path) -> Result<ProjectProfile> {
    if !path.exists() {
        return Err(ProjectIntelligenceError::MissingDirectory(
            path.display().to_string(),
        ));
    }
    if !path.is_dir() {
        return Err(ProjectIntelligenceError::NotDirectory(
            path.display().to_string(),
        ));
    }
    let root = path.canonicalize()?;
    let root_path = root
        .to_str()
        .ok_or(ProjectIntelligenceError::NonUtf8Path)?
        .to_string();
    let facts = scan_project(&root)?;
    let git = inspect_git(&root);
    let package = package_metadata(&facts);
    let package_name = package.name;
    let package_version = package.version;
    let package_description = package.description;
    let mut commands = package.commands;
    let dependencies = package.dependencies;
    let technologies = detect_technologies(&facts, &dependencies, &mut commands);
    commands.sort_by(|a, b| {
        command_order(a.kind)
            .cmp(&command_order(b.kind))
            .then_with(|| a.label.cmp(&b.label))
    });
    let languages = build_languages(&facts);
    let signals = build_signals(&facts, git.as_ref(), &commands);
    let name = package_name
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            root.file_name()
                .and_then(|value| value.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "Project".into());
    let description = package_description.or_else(|| read_readme_description(&root));
    let fingerprint = project_fingerprint(&root_path, git.as_ref(), &facts.relative_files);
    let files_seen = facts.relative_files.len();
    let test_files = facts.test_file_count;
    let documentation_files = facts.documentation_file_count;
    let entrypoints = facts.entrypoints.into_iter().collect::<Vec<_>>();

    Ok(ProjectProfile {
        schema_version: PROFILE_SCHEMA_VERSION,
        fingerprint,
        root_path,
        name,
        version: package_version,
        description,
        analyzed_at: unix_time_secs()?,
        git,
        languages,
        technologies,
        commands,
        entrypoints,
        test_paths: facts.test_paths,
        documentation: facts.documentation,
        ci_files: facts.ci_files,
        signals,
        stats: ProjectStats {
            files_seen,
            source_files: facts.source_files,
            test_files,
            documentation_files,
            sensitive_files_excluded: facts.sensitive_files_excluded,
            skipped_large_files: facts.skipped_large_files,
            todo_markers: facts.todo_markers,
            scan_truncated: facts.scan_truncated,
        },
    })
}

fn command_order(kind: ProjectCommandKind) -> u8 {
    match kind {
        ProjectCommandKind::Check => 0,
        ProjectCommandKind::Typecheck => 1,
        ProjectCommandKind::Lint => 2,
        ProjectCommandKind::Test => 3,
        ProjectCommandKind::Build => 4,
        ProjectCommandKind::Dev => 5,
        ProjectCommandKind::Other => 6,
    }
}

pub fn save_profile(conn: &Connection, profile: &ProjectProfile) -> Result<()> {
    conn.execute(
        "INSERT INTO project_profiles(root_path,fingerprint,profile_json,analyzed_at) VALUES (?1,?2,?3,?4) ON CONFLICT(root_path) DO UPDATE SET fingerprint=excluded.fingerprint,profile_json=excluded.profile_json,analyzed_at=excluded.analyzed_at",
        rusqlite::params![profile.root_path, profile.fingerprint, serde_json::to_string(profile)?, profile.analyzed_at],
    )?;
    Ok(())
}

pub fn load_profile(conn: &Connection, root_path: &str) -> Result<Option<ProjectProfile>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT profile_json FROM project_profiles WHERE root_path=?1",
            rusqlite::params![root_path],
            |row| row.get(0),
        )
        .optional()?;
    raw.map(|value| serde_json::from_str(&value).map_err(ProjectIntelligenceError::from))
        .transpose()
}

pub fn list_profiles(conn: &Connection, limit: usize) -> Result<Vec<ProjectProfile>> {
    let mut stmt = conn
        .prepare("SELECT profile_json FROM project_profiles ORDER BY analyzed_at DESC LIMIT ?1")?;
    let rows = stmt.query_map(rusqlite::params![limit.clamp(1, 500) as i64], |row| {
        row.get::<_, String>(0)
    })?;
    let mut profiles = Vec::new();
    for row in rows {
        profiles.push(serde_json::from_str(&row?)?);
    }
    Ok(profiles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use tempfile::{tempdir, NamedTempFile};

    #[test]
    fn analyzes_node_rust_project_without_reading_secrets() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::create_dir_all(dir.path().join("tests")).unwrap();
        fs::write(dir.path().join("package.json"), r#"{"name":"sample","version":"1.2.3","description":"A sample application","scripts":{"dev":"vite","build":"vite build","test":"vitest run"},"dependencies":{"react":"1","@tauri-apps/api":"2"},"devDependencies":{"typescript":"6","vite":"8","vitest":"3"}}"#).unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname='sample'\n[dependencies]\ntauri='2'\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("src/main.tsx"),
            "// TODO remove workaround\nexport const app = 1;\n",
        )
        .unwrap();
        fs::write(dir.path().join("tests/app.test.ts"), "test('x',()=>{});\n").unwrap();
        fs::write(dir.path().join(".env"), "API_KEY=do-not-read\n").unwrap();

        let profile = analyze_project(dir.path()).unwrap();
        assert_eq!(profile.name, "sample");
        assert_eq!(profile.version.as_deref(), Some("1.2.3"));
        assert!(profile.technologies.iter().any(|item| item.name == "React"));
        assert!(profile.technologies.iter().any(|item| item.name == "Tauri"));
        assert!(profile
            .commands
            .iter()
            .any(|command| command.kind == ProjectCommandKind::Test));
        assert_eq!(profile.stats.sensitive_files_excluded, 1);
        assert_eq!(profile.stats.todo_markers, 1);
    }

    #[test]
    fn gitignored_files_are_not_read_by_discovery() {
        let dir = tempdir().unwrap();
        Repository::init(dir.path()).unwrap();
        fs::write(dir.path().join(".gitignore"), "ignored.ts\n").unwrap();
        fs::write(
            dir.path().join("ignored.ts"),
            "// TODO should not be scanned\n",
        )
        .unwrap();
        fs::write(dir.path().join("main.ts"), "export const visible = true;\n").unwrap();
        let profile = analyze_project(dir.path()).unwrap();
        assert_eq!(profile.stats.todo_markers, 0);
        assert_eq!(
            profile
                .languages
                .iter()
                .find(|item| item.language == "TypeScript")
                .map(|item| item.files),
            Some(1)
        );
    }

    #[test]
    fn gitignored_directories_are_pruned_by_discovery() {
        let dir = tempdir().unwrap();
        Repository::init(dir.path()).unwrap();
        fs::write(dir.path().join(".gitignore"), "generated/\n").unwrap();
        fs::create_dir_all(dir.path().join("generated/deep")).unwrap();
        fs::write(
            dir.path().join("generated/deep/ignored.ts"),
            "// TODO should never be scanned\n",
        )
        .unwrap();
        fs::write(dir.path().join("main.ts"), "export const visible = true;\n").unwrap();

        let profile = analyze_project(dir.path()).unwrap();

        assert_eq!(profile.stats.todo_markers, 0);
        assert_eq!(
            profile
                .languages
                .iter()
                .find(|item| item.language == "TypeScript")
                .map(|item| item.files),
            Some(1)
        );
    }

    #[test]
    fn profile_round_trips_through_storage() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("README.md"),
            "# Tiny\n\nSmall but useful project description.\n",
        )
        .unwrap();
        let profile = analyze_project(dir.path()).unwrap();
        let db = NamedTempFile::new().unwrap();
        let conn = init_db(db.path()).unwrap();
        save_profile(&conn, &profile).unwrap();
        assert_eq!(
            load_profile(&conn, &profile.root_path).unwrap(),
            Some(profile.clone())
        );
        assert_eq!(list_profiles(&conn, 10).unwrap(), vec![profile]);
    }
}
