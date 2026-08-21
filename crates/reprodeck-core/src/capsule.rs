use crate::{evidence, redaction, shadow_session, timeline, workflow};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path};
use std::time::{SystemTime, SystemTimeError, UNIX_EPOCH};
use thiserror::Error;
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub const FORMAT_NAME: &str = "reprodeck";
pub const FORMAT_VERSION: u32 = 1;
const MAX_ENTRIES: usize = 2048;
const MAX_ENTRY_BYTES: u64 = 32 * 1024 * 1024;
const MAX_TOTAL_BYTES: u128 = 256 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum CapsuleError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Timeline(#[from] timeline::TimelineError),
    #[error(transparent)]
    Workflow(#[from] workflow::WorkflowError),
    #[error(transparent)]
    Evidence(#[from] evidence::EvidenceError),
    #[error(transparent)]
    Shadow(#[from] shadow_session::ShadowSessionError),
    #[error(transparent)]
    Clock(#[from] SystemTimeError),
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("unsupported capsule format: {0}")]
    UnsupportedFormat(String),
    #[error("unsupported capsule version: {0}")]
    UnsupportedVersion(u32),
    #[error("capsule entry path is unsafe: {0}")]
    UnsafePath(String),
    #[error("capsule contains an undeclared or duplicate entry: {0}")]
    UnexpectedEntry(String),
    #[error("capsule is missing a required entry: {0}")]
    MissingEntry(String),
    #[error("capsule entry is too large: {0}")]
    EntryTooLarge(String),
    #[error("capsule is too large")]
    ArchiveTooLarge,
    #[error("capsule integrity check failed for {0}")]
    Integrity(String),
    #[error("capsule must use the .reprodeck extension")]
    InvalidExtension,
}

pub type Result<T> = std::result::Result<T, CapsuleError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapsuleFile {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub media_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapsuleManifest {
    pub format: String,
    pub version: u32,
    pub created_at: i64,
    pub session_id: String,
    pub title: String,
    pub files: Vec<CapsuleFile>,
    pub redactions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapsuleSummary {
    pub session_id: String,
    pub title: String,
    pub version: u32,
    pub created_at: i64,
    pub file_count: usize,
    pub total_uncompressed_bytes: u64,
    pub redactions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapsuleExportPreview {
    pub summary: CapsuleSummary,
    pub files: Vec<CapsuleFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportedCapsule {
    pub id: String,
    pub source_path: String,
    pub stored_path: String,
    pub session_id: Option<String>,
    pub title: Option<String>,
    pub format_version: u32,
    pub sha256: String,
    pub imported_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CapsuleTimelineEntry {
    action: timeline::ActionRecord,
    execution: Option<timeline::ExecutionRecord>,
    receipt: Option<timeline::ReceiptRecord>,
    artifacts: Vec<evidence::ArtifactRecord>,
}

fn unix_time_secs() -> Result<i64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64)
}

fn sha256(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn require_extension(path: &Path) -> Result<()> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !extension.eq_ignore_ascii_case("reprodeck") {
        return Err(CapsuleError::InvalidExtension);
    }
    Ok(())
}

fn validate_relative_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.contains('\\')
        || name.contains('\0')
        || name
            .split('/')
            .next()
            .is_some_and(|segment| segment.ends_with(':'))
    {
        return Err(CapsuleError::UnsafePath(name.to_string()));
    }
    let path = Path::new(name);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(CapsuleError::UnsafePath(name.to_string()));
    }
    Ok(())
}

fn add_json<T: Serialize>(
    entries: &mut BTreeMap<String, Vec<u8>>,
    path: &str,
    value: &T,
) -> Result<()> {
    validate_relative_name(path)?;
    let bytes = serde_json::to_vec_pretty(value)?;
    if bytes.len() as u64 > MAX_ENTRY_BYTES {
        return Err(CapsuleError::EntryTooLarge(path.to_string()));
    }
    entries.insert(path.to_string(), bytes);
    Ok(())
}

fn build_timeline(conn: &Connection, session_id: &str) -> Result<Vec<CapsuleTimelineEntry>> {
    let actions = timeline::list_actions(conn, session_id, None, 500)?;
    let mut result = Vec::with_capacity(actions.len());
    for action in actions {
        let execution = timeline::list_executions(conn, &action.id)?
            .into_iter()
            .last();
        let receipt = if let Some(execution) = execution.as_ref() {
            timeline::list_receipts(conn, &execution.id)?
                .into_iter()
                .last()
        } else {
            None
        };
        let artifacts = if let Some(receipt) = receipt.as_ref() {
            evidence::list_artifacts_for_receipt(conn, &receipt.id)?
        } else {
            Vec::new()
        };
        result.push(CapsuleTimelineEntry {
            action,
            execution,
            receipt,
            artifacts,
        });
    }
    Ok(result)
}

fn sanitized_session(session: &timeline::SessionRecord) -> serde_json::Value {
    let meta = workflow::session_meta(session);
    serde_json::json!({
        "id": session.id,
        "created_at": session.created_at,
        "updated_at": session.updated_at,
        "state": session.state,
        "title": redaction::redact_text(&meta.title),
        "expected": redaction::redact_text(&meta.expected),
        "actual": redaction::redact_text(&meta.actual),
        "notes": redaction::redact_text(&meta.notes),
    })
}

struct PreparedCapsule {
    manifest: CapsuleManifest,
    entries: BTreeMap<String, Vec<u8>>,
}

fn prepare_session_export(
    conn: &Connection,
    artifact_store: &Path,
    session_id: &str,
) -> Result<PreparedCapsule> {
    let session = timeline::get_session_record(conn, session_id)?
        .ok_or_else(|| CapsuleError::SessionNotFound(session_id.to_string()))?;
    let meta = workflow::session_meta(&session);
    let mut entries = BTreeMap::<String, Vec<u8>>::new();
    let mut redactions = Vec::new();

    add_json(&mut entries, "session.json", &sanitized_session(&session))?;
    add_json(
        &mut entries,
        "environment.json",
        &workflow::latest_environment(conn, session_id)?,
    )?;
    add_json(
        &mut entries,
        "reproduction.json",
        &serde_json::json!({
            "steps": workflow::list_reproduction_steps(conn, session_id)?,
            "runs": workflow::list_reproduction_runs(conn, session_id)?,
            "outcome": workflow::outcome_for_session(conn, session_id)?,
        }),
    )?;

    let timeline_entries = build_timeline(conn, session_id)?;
    add_json(&mut entries, "timeline.json", &timeline_entries)?;
    let evidence_items = evidence::list_evidence_items(conn, session_id, 1000)?;
    add_json(&mut entries, "evidence/index.json", &evidence_items)?;

    let mut exported_artifacts = BTreeSet::new();
    for entry in &timeline_entries {
        for artifact in &entry.artifacts {
            if !exported_artifacts.insert(artifact.id.clone()) {
                continue;
            }
            if artifact.size < 0 || artifact.size as u64 > MAX_ENTRY_BYTES {
                redactions.push(format!(
                    "artifact {} omitted because it exceeds the per-file limit",
                    artifact.id
                ));
                continue;
            }
            let bytes = evidence::read_artifact(conn, artifact_store, &artifact.id)?;
            let path = format!("evidence/artifacts/{}", artifact.id);
            validate_relative_name(&path)?;
            entries.insert(path, bytes);
        }
    }

    let mut exported_named_diff = false;
    if shadow_session::get_session_shadow(conn, session_id)?.is_some() {
        let diff = shadow_session::session_shadow_diff(conn, session_id)?;
        if !diff.patch.is_empty() {
            let denied_path = diff.files.iter().find(|path| {
                matches!(
                    redaction::redact_path(Path::new(path)),
                    redaction::RedactionResult::Redacted { .. }
                        | redaction::RedactionResult::Excluded { .. }
                )
            });
            if let Some(path) = denied_path {
                redactions.push(format!(
                    "shadow diff omitted because it references sensitive path: {path}"
                ));
            } else {
                let sanitized = redaction::redact_text(&diff.patch);
                entries.insert("diffs/shadow.patch".into(), sanitized.into_bytes());
                exported_named_diff = true;
            }
        }
    }

    if !exported_named_diff {
        if let Some(item) = evidence_items.iter().find(|item| {
            item.kind == evidence::EvidenceKind::GitDiff.as_str() && item.artifact_id.is_some()
        }) {
            if let Some(artifact_id) = item.artifact_id.as_deref() {
                let bytes = evidence::read_artifact(conn, artifact_store, artifact_id)?;
                if bytes.len() as u64 <= MAX_ENTRY_BYTES {
                    entries.insert("diffs/verified.patch".into(), bytes);
                }
            }
        }
    }

    let total_payload: u128 = entries.values().map(|value| value.len() as u128).sum();
    if entries.len() > MAX_ENTRIES || total_payload > MAX_TOTAL_BYTES {
        return Err(CapsuleError::ArchiveTooLarge);
    }

    let files = entries
        .iter()
        .map(|(path, bytes)| CapsuleFile {
            path: path.clone(),
            sha256: sha256(bytes),
            size: bytes.len() as u64,
            media_type: if path.ends_with(".json") {
                "application/json".to_string()
            } else if path.ends_with(".patch") {
                "text/x-diff".to_string()
            } else {
                "application/octet-stream".to_string()
            },
        })
        .collect::<Vec<_>>();

    let manifest = CapsuleManifest {
        format: FORMAT_NAME.to_string(),
        version: FORMAT_VERSION,
        created_at: unix_time_secs()?,
        session_id: session.id.clone(),
        title: redaction::redact_text(if meta.title.is_empty() {
            &session.id
        } else {
            &meta.title
        }),
        files,
        redactions,
    };

    Ok(PreparedCapsule { manifest, entries })
}

fn summary_from_manifest(manifest: &CapsuleManifest) -> CapsuleSummary {
    CapsuleSummary {
        session_id: manifest.session_id.clone(),
        title: manifest.title.clone(),
        version: manifest.version,
        created_at: manifest.created_at,
        file_count: manifest.files.len(),
        total_uncompressed_bytes: manifest.files.iter().map(|file| file.size).sum(),
        redactions: manifest.redactions.clone(),
    }
}

pub fn preview_session_export(
    conn: &Connection,
    artifact_store: &Path,
    session_id: &str,
) -> Result<CapsuleExportPreview> {
    let prepared = prepare_session_export(conn, artifact_store, session_id)?;
    Ok(CapsuleExportPreview {
        summary: summary_from_manifest(&prepared.manifest),
        files: prepared.manifest.files,
    })
}

pub fn export_session(
    conn: &Connection,
    artifact_store: &Path,
    session_id: &str,
    destination: &Path,
) -> Result<CapsuleSummary> {
    require_extension(destination)?;
    let prepared = prepare_session_export(conn, artifact_store, session_id)?;
    let manifest = prepared.manifest;
    let entries = prepared.entries;

    let checksums = manifest
        .files
        .iter()
        .map(|file| (file.path.clone(), file.sha256.clone()))
        .collect::<BTreeMap<_, _>>();
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    let checksums_bytes = serde_json::to_vec_pretty(&checksums)?;

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = destination.with_extension(format!("reprodeck.{}.tmp", Uuid::new_v4()));
    let file = File::create(&temp)?;
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o600);

    writer.start_file("manifest.json", options)?;
    writer.write_all(&manifest_bytes)?;
    writer.start_file("checksums.json", options)?;
    writer.write_all(&checksums_bytes)?;
    for (path, bytes) in entries {
        writer.start_file(path, options)?;
        writer.write_all(&bytes)?;
    }
    writer.finish()?;

    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(&temp, destination).inspect_err(|_| {
        let _ = fs::remove_file(&temp);
    })?;

    Ok(summary_from_manifest(&manifest))
}

fn read_entry_bytes<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> Result<Vec<u8>> {
    let mut file = archive.by_name(name)?;
    if file.is_dir() || file.is_symlink() || file.size() > MAX_ENTRY_BYTES {
        return Err(CapsuleError::EntryTooLarge(name.to_string()));
    }
    let safe = file
        .enclosed_name()
        .ok_or_else(|| CapsuleError::UnsafePath(name.to_string()))?;
    if safe != Path::new(name) {
        return Err(CapsuleError::UnsafePath(name.to_string()));
    }
    let mut bytes = Vec::with_capacity(file.size().min(MAX_ENTRY_BYTES) as usize);
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn inspect_reader<R: Read + Seek>(reader: R) -> Result<CapsuleSummary> {
    let mut archive = ZipArchive::new(reader)?;
    if archive.len() > MAX_ENTRIES
        || archive
            .decompressed_size()
            .is_some_and(|size| size > MAX_TOTAL_BYTES)
    {
        return Err(CapsuleError::ArchiveTooLarge);
    }

    let manifest_bytes = read_entry_bytes(&mut archive, "manifest.json")?;
    let manifest: CapsuleManifest = serde_json::from_slice(&manifest_bytes)?;
    if manifest.format != FORMAT_NAME {
        return Err(CapsuleError::UnsupportedFormat(manifest.format));
    }
    if manifest.version != FORMAT_VERSION {
        return Err(CapsuleError::UnsupportedVersion(manifest.version));
    }
    if manifest.files.len() > MAX_ENTRIES.saturating_sub(2) {
        return Err(CapsuleError::ArchiveTooLarge);
    }

    let checksums_bytes = read_entry_bytes(&mut archive, "checksums.json")?;
    let checksums: BTreeMap<String, String> = serde_json::from_slice(&checksums_bytes)?;
    let manifest_checksums = manifest
        .files
        .iter()
        .map(|file| (file.path.clone(), file.sha256.clone()))
        .collect::<BTreeMap<_, _>>();
    if checksums != manifest_checksums {
        return Err(CapsuleError::Integrity("checksums.json".to_string()));
    }

    let mut declared = BTreeSet::new();
    declared.insert("manifest.json".to_string());
    declared.insert("checksums.json".to_string());
    for file in &manifest.files {
        validate_relative_name(&file.path)?;
        if file.size > MAX_ENTRY_BYTES || !declared.insert(file.path.clone()) {
            return Err(CapsuleError::UnexpectedEntry(file.path.clone()));
        }
    }

    let mut actual = BTreeSet::new();
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        if file.is_dir() || file.is_symlink() {
            return Err(CapsuleError::UnsafePath(file.name().to_string()));
        }
        let name = file.name().to_string();
        validate_relative_name(&name)?;
        if file.enclosed_name().as_deref() != Some(Path::new(&name)) {
            return Err(CapsuleError::UnsafePath(name));
        }
        if !actual.insert(name.clone()) || !declared.contains(&name) {
            return Err(CapsuleError::UnexpectedEntry(name));
        }
    }
    for name in &declared {
        if !actual.contains(name) {
            return Err(CapsuleError::MissingEntry(name.clone()));
        }
    }

    let mut total = 0u128;
    for file in &manifest.files {
        let bytes = read_entry_bytes(&mut archive, &file.path)?;
        total += bytes.len() as u128;
        if total > MAX_TOTAL_BYTES {
            return Err(CapsuleError::ArchiveTooLarge);
        }
        if bytes.len() as u64 != file.size || sha256(&bytes) != file.sha256 {
            return Err(CapsuleError::Integrity(file.path.clone()));
        }
    }

    Ok(CapsuleSummary {
        session_id: manifest.session_id,
        title: manifest.title,
        version: manifest.version,
        created_at: manifest.created_at,
        file_count: manifest.files.len(),
        total_uncompressed_bytes: total.min(u64::MAX as u128) as u64,
        redactions: manifest.redactions,
    })
}

pub fn inspect_capsule(path: &Path) -> Result<CapsuleSummary> {
    require_extension(path)?;
    inspect_reader(File::open(path)?)
}

pub fn import_capsule(
    conn: &Connection,
    source: &Path,
    library_dir: &Path,
) -> Result<ImportedCapsule> {
    require_extension(source)?;
    let mut source_file = File::open(source)?;
    let summary = inspect_reader(&mut source_file)?;
    // Validate and copy from the same open file handle. This avoids a TOCTOU window
    // where the path could be swapped after validation but before persistence.
    source_file.seek(SeekFrom::Start(0))?;
    let max_source_bytes = (MAX_TOTAL_BYTES as u64).saturating_add(32 * 1024 * 1024);
    if source_file.metadata()?.len() > max_source_bytes {
        return Err(CapsuleError::ArchiveTooLarge);
    }
    let mut bytes = Vec::new();
    source_file
        .take(max_source_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_source_bytes {
        return Err(CapsuleError::ArchiveTooLarge);
    }
    fs::create_dir_all(library_dir)?;
    let digest = sha256(&bytes);
    let id = Uuid::new_v4().to_string();
    let stored = library_dir.join(format!("{id}.reprodeck"));
    let temp = library_dir.join(format!(".{id}.importing"));
    {
        let mut output = File::create(&temp)?;
        output.write_all(&bytes)?;
        output.sync_all()?;
    }
    fs::rename(&temp, &stored).inspect_err(|_| {
        let _ = fs::remove_file(&temp);
    })?;

    let imported_at = unix_time_secs()?;
    let source_text = source.to_string_lossy().into_owned();
    let stored_text = stored.to_string_lossy().into_owned();
    if let Err(error) = conn.execute(
        "INSERT INTO imported_capsules(id,source_path,stored_path,session_id,title,format_version,sha256,imported_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        rusqlite::params![id,source_text,stored_text,summary.session_id,summary.title,summary.version, digest, imported_at],
    ) {
        let _ = fs::remove_file(&stored);
        return Err(CapsuleError::Db(error));
    }
    Ok(ImportedCapsule {
        id,
        source_path: source_text,
        stored_path: stored_text,
        session_id: Some(summary.session_id),
        title: Some(summary.title),
        format_version: summary.version,
        sha256: digest,
        imported_at,
    })
}

pub fn list_imported_capsules(conn: &Connection) -> Result<Vec<ImportedCapsule>> {
    let mut stmt = conn.prepare(
        "SELECT id,source_path,stored_path,session_id,title,format_version,sha256,imported_at FROM imported_capsules ORDER BY imported_at DESC,rowid DESC LIMIT 500",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ImportedCapsule {
                id: row.get(0)?,
                source_path: row.get(1)?,
                stored_path: row.get(2)?,
                session_id: row.get(3)?,
                title: row.get(4)?,
                format_version: row.get::<_, i64>(5)?.max(0) as u32,
                sha256: row.get(6)?,
                imported_at: row.get(7)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_imported_capsule(conn: &Connection, id: &str) -> Result<Option<ImportedCapsule>> {
    Ok(conn.query_row(
        "SELECT id,source_path,stored_path,session_id,title,format_version,sha256,imported_at FROM imported_capsules WHERE id=?1",
        rusqlite::params![id],
        |row| Ok(ImportedCapsule {
            id: row.get(0)?, source_path: row.get(1)?, stored_path: row.get(2)?, session_id: row.get(3)?, title: row.get(4)?,
            format_version: row.get::<_, i64>(5)?.max(0) as u32, sha256: row.get(6)?, imported_at: row.get(7)?,
        }),
    ).optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use std::io::Cursor;
    use tempfile::{tempdir, NamedTempFile};

    #[test]
    fn unsafe_names_are_rejected() {
        for name in ["../x", "/x", "C:/x", "a\\b", ""] {
            assert!(validate_relative_name(name).is_err(), "{name}");
        }
        assert!(validate_relative_name("evidence/a.txt").is_ok());
    }

    #[test]
    fn import_rejects_undeclared_entries() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad.reprodeck");
        let manifest = CapsuleManifest {
            format: FORMAT_NAME.into(),
            version: FORMAT_VERSION,
            created_at: 1,
            session_id: "s".into(),
            title: "t".into(),
            files: vec![],
            redactions: vec![],
        };
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default();
        writer.start_file("manifest.json", opts).unwrap();
        writer
            .write_all(&serde_json::to_vec(&manifest).unwrap())
            .unwrap();
        writer.start_file("checksums.json", opts).unwrap();
        writer.write_all(b"{}").unwrap();
        writer.start_file("surprise.txt", opts).unwrap();
        writer.write_all(b"nope").unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        fs::write(&path, bytes).unwrap();
        assert!(matches!(
            inspect_capsule(&path),
            Err(CapsuleError::UnexpectedEntry(_))
        ));
    }

    #[test]
    fn export_inspect_and_import_round_trip() {
        let dir = tempdir().unwrap();
        let db_file = NamedTempFile::new().unwrap();
        let conn = init_db(db_file.path()).unwrap();
        workflow::create_bug_session(
            &conn,
            "session-1",
            &workflow::SessionMeta {
                title: "Capsule round trip".into(),
                expected: "PASS".into(),
                actual: "FAIL".into(),
                notes: "token=ghp_abcdefghijklmnopqrstuvwxyz123456".into(),
            },
        )
        .unwrap();
        let path = dir.path().join("session.reprodeck");
        let artifacts = dir.path().join("artifacts");
        let preview = preview_session_export(&conn, &artifacts, "session-1").unwrap();
        assert!(preview.files.iter().any(|file| file.path == "session.json"));
        assert!(preview
            .files
            .iter()
            .any(|file| file.path == "timeline.json"));
        let exported = export_session(&conn, &artifacts, "session-1", &path).unwrap();
        assert_eq!(exported.session_id, "session-1");
        assert_eq!(exported.version, FORMAT_VERSION);

        let inspected = inspect_capsule(&path).unwrap();
        assert_eq!(inspected.session_id, exported.session_id);
        assert_eq!(inspected.title, exported.title);
        assert_eq!(inspected.file_count, exported.file_count);

        let library = dir.path().join("library");
        let imported = import_capsule(&conn, &path, &library).unwrap();
        assert!(Path::new(&imported.stored_path).is_file());
        assert_eq!(imported.session_id.as_deref(), Some("session-1"));
        assert_eq!(list_imported_capsules(&conn).unwrap().len(), 1);
    }

    #[test]
    fn tampered_payload_is_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("tampered.reprodeck");
        let payload = b"original".to_vec();
        let file = CapsuleFile {
            path: "session.json".into(),
            sha256: sha256(&payload),
            size: payload.len() as u64,
            media_type: "application/json".into(),
        };
        let manifest = CapsuleManifest {
            format: FORMAT_NAME.into(),
            version: FORMAT_VERSION,
            created_at: 1,
            session_id: "s".into(),
            title: "t".into(),
            files: vec![file.clone()],
            redactions: vec![],
        };
        let checksums = BTreeMap::from([(file.path.clone(), file.sha256.clone())]);
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default();
        writer.start_file("manifest.json", opts).unwrap();
        writer
            .write_all(&serde_json::to_vec(&manifest).unwrap())
            .unwrap();
        writer.start_file("checksums.json", opts).unwrap();
        writer
            .write_all(&serde_json::to_vec(&checksums).unwrap())
            .unwrap();
        writer.start_file("session.json", opts).unwrap();
        writer.write_all(b"tampered").unwrap();
        fs::write(&path, writer.finish().unwrap().into_inner()).unwrap();
        assert!(matches!(
            inspect_capsule(&path),
            Err(CapsuleError::Integrity(_))
        ));
    }

    #[test]
    fn imported_capsule_table_round_trips_empty_list() {
        let file = NamedTempFile::new().unwrap();
        let conn = init_db(file.path()).unwrap();
        assert!(list_imported_capsules(&conn).unwrap().is_empty());
    }
}
