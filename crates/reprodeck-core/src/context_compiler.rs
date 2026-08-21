use crate::git_ignore::GitIgnoreMatcher;
use crate::redaction::{self, RedactionResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Reverse;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path};
use thiserror::Error;
use walkdir::{DirEntry, WalkDir};

const MAX_CANDIDATES: usize = 20_000;
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_READ_BYTES: usize = 256 * 1024;

#[derive(Debug, Error)]
pub enum ContextCompilerError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("project path is not a directory: {0}")]
    NotDirectory(String),
    #[error("project path is not valid UTF-8")]
    NonUtf8Path,
}

pub type Result<T> = std::result::Result<T, ContextCompilerError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextRequest {
    pub query: String,
    pub max_files: usize,
    pub max_chars: usize,
}

impl ContextRequest {
    pub fn bounded(query: impl Into<String>, max_files: usize, max_chars: usize) -> Self {
        Self {
            query: query.into(),
            max_files: max_files.clamp(1, 40),
            max_chars: max_chars.clamp(4_000, 120_000),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextPacket {
    pub root_path: String,
    pub query: String,
    pub snippets: Vec<ContextSnippet>,
    pub stats: ContextStats,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextSnippet {
    pub id: String,
    pub path: String,
    pub language: String,
    pub score: i64,
    pub reasons: Vec<String>,
    pub line_start: usize,
    pub line_end: usize,
    pub content: String,
    pub checksum: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ContextStats {
    pub files_considered: usize,
    pub files_ranked: usize,
    pub sensitive_files_excluded: usize,
    pub skipped_large_or_binary: usize,
    pub selected_chars: usize,
    pub candidate_scan_truncated: bool,
    pub packet_truncated: bool,
}

#[derive(Debug)]
struct RankedFile {
    path: String,
    score: i64,
    reasons: Vec<String>,
    text: String,
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

fn is_sensitive(path: &Path) -> bool {
    if path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .any(|segment| matches!(segment.as_str(), ".ssh" | ".aws" | ".gnupg" | ".azure"))
    {
        return true;
    }
    !matches!(redaction::redact_path(path), RedactionResult::Included(_))
}

fn language_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "rs" => "rust",
        "ts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "jsx",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "cs" => "csharp",
        "php" => "php",
        "rb" => "ruby",
        "swift" => "swift",
        "c" | "h" => "c",
        "cc" | "cpp" | "hpp" => "cpp",
        "vue" => "vue",
        "svelte" => "svelte",
        "html" => "html",
        "css" | "scss" | "sass" | "less" => "css",
        "toml" => "toml",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "md" | "mdx" => "markdown",
        "sql" => "sql",
        "sh" => "shell",
        "ps1" => "powershell",
        _ => "text",
    }
}

fn eligible(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(
        name.as_str(),
        "dockerfile" | "makefile" | "justfile" | "procfile"
    ) {
        return true;
    }
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
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

fn tokenize(query: &str) -> Vec<String> {
    let mut tokens = BTreeSet::new();
    let mut current = String::new();
    for ch in query.chars() {
        if ch.is_alphanumeric() || ch == '_' || ch == '-' {
            current.extend(ch.to_lowercase());
        } else if current.len() >= 2 {
            tokens.insert(std::mem::take(&mut current));
        } else {
            current.clear();
        }
    }
    if current.len() >= 2 {
        tokens.insert(current);
    }
    tokens
        .into_iter()
        .filter(|token| {
            !matches!(
                token.as_str(),
                "the"
                    | "and"
                    | "for"
                    | "with"
                    | "from"
                    | "this"
                    | "that"
                    | "как"
                    | "что"
                    | "это"
                    | "для"
                    | "или"
                    | "при"
                    | "где"
                    | "почему"
            )
        })
        .collect()
}

fn base_path_score(path: &str) -> i64 {
    let lower = path.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "readme.md" | "package.json" | "cargo.toml" | "pyproject.toml" | "go.mod"
    ) {
        18
    } else if lower.ends_with("/main.rs")
        || lower.ends_with("/main.ts")
        || lower.ends_with("/main.tsx")
        || lower.ends_with("/index.ts")
        || lower.ends_with("/index.tsx")
    {
        14
    } else if lower.contains("/test") || lower.contains(".test.") || lower.contains(".spec.") {
        7
    } else {
        0
    }
}

fn rank(path: &str, text: &str, tokens: &[String]) -> (i64, Vec<String>) {
    let path_lower = path.to_ascii_lowercase();
    let text_lower = text.to_lowercase();
    let mut score = base_path_score(path);
    let mut reasons = Vec::new();
    if score > 0 {
        reasons.push("project anchor".into());
    }
    for token in tokens {
        if path_lower.contains(token) {
            score += 24;
            reasons.push(format!("path matches '{token}'"));
        }
        let count = text_lower.matches(token).take(20).count() as i64;
        if count > 0 {
            score += 3 + count.min(12) * 2;
            reasons.push(format!("content matches '{token}'"));
        }
    }
    reasons.sort();
    reasons.dedup();
    (score, reasons)
}

fn read_candidate(path: &Path) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    let slice = if bytes.len() > MAX_READ_BYTES {
        &bytes[..MAX_READ_BYTES]
    } else {
        &bytes
    };
    Some(String::from_utf8_lossy(slice).into_owned())
}

fn best_window(text: &str, tokens: &[String], max_chars: usize) -> (usize, usize, String, bool) {
    let lines = text.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return (1, 1, String::new(), false);
    }
    let mut best_line = 0usize;
    let mut best_hits = 0usize;
    for (index, line) in lines.iter().enumerate() {
        let lower = line.to_lowercase();
        let hits = tokens
            .iter()
            .filter(|token| lower.contains(token.as_str()))
            .count();
        if hits > best_hits {
            best_hits = hits;
            best_line = index;
        }
    }
    let radius = if best_hits > 0 { 8 } else { 14 };
    let start = best_line.saturating_sub(radius);
    let end = (best_line + radius + 1).min(lines.len());
    let mut output = String::new();
    let mut actual_end = start;
    let mut truncated = start > 0 || end < lines.len();
    for (index, line) in lines[start..end].iter().enumerate() {
        let line_number = start + index + 1;
        let rendered = format!("{line_number:>5} | {line}\n");
        if output.chars().count() + rendered.chars().count() > max_chars {
            truncated = true;
            break;
        }
        output.push_str(&rendered);
        actual_end = start + index;
    }
    (
        start + 1,
        actual_end + 1,
        redaction::redact_text(&output),
        truncated,
    )
}

fn snippet_id(path: &str, checksum: &str, start: usize, end: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.as_bytes());
    hasher.update([0]);
    hasher.update(checksum.as_bytes());
    hasher.update([0]);
    hasher.update(start.to_string().as_bytes());
    hasher.update([0]);
    hasher.update(end.to_string().as_bytes());
    format!(
        "ctx:{}:{}",
        &hex::encode(hasher.finalize())[..10],
        path.replace(['/', '\\'], ":")
    )
}

pub fn compile_context(project_root: &Path, request: &ContextRequest) -> Result<ContextPacket> {
    if !project_root.is_dir() {
        return Err(ContextCompilerError::NotDirectory(
            project_root.display().to_string(),
        ));
    }
    let root = project_root.canonicalize()?;
    let root_path = root
        .to_str()
        .ok_or(ContextCompilerError::NonUtf8Path)?
        .to_string();
    let tokens = tokenize(&request.query);
    let mut ranked = Vec::new();
    let mut stats = ContextStats::default();
    let mut file_entries_seen = 0usize;

    let git_ignore = GitIgnoreMatcher::discover(&root);
    for entry in WalkDir::new(&root)
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
        if file_entries_seen > MAX_CANDIDATES {
            stats.candidate_scan_truncated = true;
            break;
        }
        let Some(relative) = relative_text(&root, entry.path()) else {
            continue;
        };
        let relative_path = Path::new(&relative);
        if is_sensitive(relative_path) {
            stats.sensitive_files_excluded += 1;
            continue;
        }
        if !eligible(relative_path) {
            continue;
        }
        stats.files_considered += 1;
        let Some(text) = read_candidate(entry.path()) else {
            stats.skipped_large_or_binary += 1;
            continue;
        };
        let (score, reasons) = rank(&relative, &text, &tokens);
        if score > 0 || tokens.is_empty() {
            ranked.push(RankedFile {
                path: relative,
                score,
                reasons,
                text,
            });
        }
    }
    ranked.sort_by_key(|item| (Reverse(item.score), item.path.clone()));
    stats.files_ranked = ranked.len();

    let max_files = request.max_files.clamp(1, 40);
    let max_chars = request.max_chars.clamp(4_000, 120_000);
    let per_file = (max_chars / max_files.max(1)).clamp(1_500, 16_000);
    let mut snippets = Vec::new();
    let mut selected_chars = 0usize;
    for item in ranked.into_iter().take(max_files * 3) {
        if snippets.len() >= max_files || selected_chars >= max_chars {
            break;
        }
        let remaining = max_chars - selected_chars;
        if remaining < 400 {
            stats.packet_truncated = true;
            break;
        }
        let budget = per_file.min(remaining);
        let checksum = hex::encode(Sha256::digest(item.text.as_bytes()));
        let (line_start, line_end, content, truncated) = best_window(&item.text, &tokens, budget);
        if content.trim().is_empty() {
            continue;
        }
        let chars = content.chars().count();
        selected_chars += chars;
        snippets.push(ContextSnippet {
            id: snippet_id(&item.path, &checksum, line_start, line_end),
            path: item.path.clone(),
            language: language_for(Path::new(&item.path)).into(),
            score: item.score,
            reasons: item.reasons,
            line_start,
            line_end,
            content,
            checksum,
            truncated,
        });
    }
    stats.selected_chars = selected_chars;
    stats.packet_truncated |= snippets.len() >= max_files && stats.files_ranked > snippets.len();
    Ok(ContextPacket {
        root_path,
        query: request.query.clone(),
        snippets,
        stats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn ranks_relevant_files_and_excludes_secret_paths() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src/auth")).unwrap();
        fs::write(
            dir.path().join("src/auth/session.ts"),
            "export function refreshToken() { return retryWithCachedHeader(); }\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("src/other.ts"),
            "export const unrelated = 1;\n",
        )
        .unwrap();
        fs::write(
            dir.path().join(".env"),
            "TOKEN=super-secret-token-value-that-must-never-be-ranked\n",
        )
        .unwrap();
        let request = ContextRequest::bounded("refresh token cached header", 5, 8_000);
        let packet = compile_context(dir.path(), &request).unwrap();
        assert_eq!(
            packet.snippets.first().map(|item| item.path.as_str()),
            Some("src/auth/session.ts")
        );
        assert_eq!(packet.stats.sensitive_files_excluded, 1);
        assert!(packet
            .snippets
            .iter()
            .all(|item| !item.content.contains("super-secret")));
    }

    #[test]
    fn gitignored_files_are_not_context_candidates() {
        let dir = tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        fs::write(dir.path().join(".gitignore"), "ignored.ts\n").unwrap();
        fs::write(
            dir.path().join("ignored.ts"),
            "refresh token secret implementation\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("visible.ts"),
            "refresh token visible implementation\n",
        )
        .unwrap();
        let request = ContextRequest::bounded("refresh token", 5, 8_000);
        let packet = compile_context(dir.path(), &request).unwrap();
        assert!(packet.snippets.iter().any(|item| item.path == "visible.ts"));
        assert!(packet.snippets.iter().all(|item| item.path != "ignored.ts"));
    }

    #[test]
    fn gitignored_directories_are_pruned_from_context_candidates() {
        let dir = tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        fs::write(dir.path().join(".gitignore"), "generated/\n").unwrap();
        fs::create_dir_all(dir.path().join("generated/deep")).unwrap();
        fs::write(
            dir.path().join("generated/deep/ignored.ts"),
            "refresh token should never be considered\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("visible.ts"),
            "refresh token visible implementation\n",
        )
        .unwrap();

        let packet = compile_context(
            dir.path(),
            &ContextRequest::bounded("refresh token", 5, 8_000),
        )
        .unwrap();

        assert!(packet.snippets.iter().any(|item| item.path == "visible.ts"));
        assert!(packet
            .snippets
            .iter()
            .all(|item| !item.path.starts_with("generated/")));
    }

    #[test]
    fn packet_respects_file_and_character_budgets() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        for index in 0..20 {
            fs::write(
                dir.path().join(format!("src/thing{index}.rs")),
                format!("fn token_refresh_{index}() {{}}\n"),
            )
            .unwrap();
        }
        let request = ContextRequest::bounded("token refresh", 3, 4_000);
        let packet = compile_context(dir.path(), &request).unwrap();
        assert!(packet.snippets.len() <= 3);
        assert!(packet.stats.selected_chars <= 4_000);
    }

    #[test]
    fn stable_context_ids_change_when_content_changes() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), "fn refresh_token() {}\n").unwrap();
        let request = ContextRequest::bounded("refresh token", 2, 4_000);
        let first = compile_context(dir.path(), &request).unwrap();
        fs::write(
            dir.path().join("main.rs"),
            "fn refresh_token() { println!(\"changed\"); }\n",
        )
        .unwrap();
        let second = compile_context(dir.path(), &request).unwrap();
        assert_ne!(first.snippets[0].id, second.snippets[0].id);
    }
}
