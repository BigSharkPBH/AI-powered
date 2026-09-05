use std::{fs, path::Path};

use chrono::{TimeZone, Utc};
use rusqlite::{Connection, OpenFlags};

use super::{
    LegacyKind, LegacySearchRoot, backup_legacy, backup_legacy_into_app_data, detect_legacy_roots,
    legacy_migration_status, legacy_search_roots, secret_slots_configured, switch_legacy_archive,
    try_first_run_legacy_migration, verify_legacy_archive,
};
use crate::config::{AppConfigV1, ConfigDirs};

const SECRET_DECOY: &str = "sk-live-migrate-forbidden-key";
const SAFE_CONFIG: &str = r#"{"configVersion":1}"#;

fn write(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

fn write_bytes(path: &Path, body: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

fn write_ok_sqlite(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch("CREATE TABLE notes(body TEXT); INSERT INTO notes VALUES ('local note');")
        .unwrap();
}

fn write_account_sqlite(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE accounts(email TEXT, password TEXT);
             INSERT INTO accounts VALUES ('user@example.com', 'not-migrated');",
        )
        .unwrap();
}

#[allow(clippy::permissions_set_readonly_false)]
fn allow_cleanup(path: &Path) {
    if path.is_dir()
        && let Ok(entries) = fs::read_dir(path)
    {
        for entry in entries.flatten() {
            allow_cleanup(&entry.path());
        }
    }
    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        permissions.set_readonly(false);
        let _ = fs::set_permissions(path, permissions);
    }
}

fn contains_secret_material(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "password",
        "secretvalue",
        "secretcontents",
        "\"token\"",
        "control_api_token",
        "desktop_session",
        SECRET_DECOY,
    ]
    .iter()
    .any(|needle| lower.contains(&needle.to_ascii_lowercase()))
}

#[test]
fn detects_repo_dev_tree_from_injected_repository() {
    let repo = tempfile::tempdir().unwrap();
    write(&repo.path().join("config/local.json"), SAFE_CONFIG);
    write_ok_sqlite(&repo.path().join("data/app.sqlite"));
    fs::create_dir_all(repo.path().join("data/materials")).unwrap();
    fs::create_dir_all(repo.path().join("data/avatar")).unwrap();

    let found = detect_legacy_roots(&[LegacySearchRoot::Repository(repo.path().to_path_buf())]);

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].kind, LegacyKind::Repository);
    assert_eq!(found[0].path, repo.path());
    assert_eq!(
        found[0].config_path.as_deref(),
        Some(repo.path().join("config/local.json").as_path())
    );
    assert_eq!(
        found[0].sqlite_path.as_deref(),
        Some(repo.path().join("data/app.sqlite").as_path())
    );
    assert_eq!(
        found[0].materials_path.as_deref(),
        Some(repo.path().join("data/materials").as_path())
    );
}

#[test]
fn detects_electron_userdata_under_injected_appdata_only() {
    let app_data = tempfile::tempdir().unwrap();
    write(
        &app_data.path().join("AI Virtual Assistant/config.json"),
        SAFE_CONFIG,
    );
    write(
        &app_data
            .path()
            .join("authorized-interview-screen-helper/local.json"),
        SAFE_CONFIG,
    );
    write_ok_sqlite(
        &app_data
            .path()
            .join("authorized-interview-screen-helper/app.sqlite3"),
    );

    let found = detect_legacy_roots(&[LegacySearchRoot::AppData(app_data.path().to_path_buf())]);

    assert_eq!(found.len(), 2);
    assert!(
        found
            .iter()
            .all(|root| root.kind == LegacyKind::ElectronUserData)
    );
    assert!(
        found
            .iter()
            .all(|root| root.path.starts_with(app_data.path()))
    );
    assert!(found.iter().any(|root| {
        root.path.ends_with("AI Virtual Assistant")
            && root.config_path.as_ref().is_some_and(|path| {
                path.ends_with(Path::new("AI Virtual Assistant").join("config.json"))
            })
    }));
    assert!(found.iter().any(|root| {
        root.path.ends_with("authorized-interview-screen-helper")
            && root.sqlite_path.as_ref().is_some_and(|path| {
                path.ends_with(Path::new("authorized-interview-screen-helper").join("app.sqlite3"))
            })
    }));
}

#[test]
fn empty_injected_roots_yield_no_legacy_trees() {
    let empty = tempfile::tempdir().unwrap();
    let found = detect_legacy_roots(&[
        LegacySearchRoot::Repository(empty.path().to_path_buf()),
        LegacySearchRoot::AppData(empty.path().to_path_buf()),
    ]);
    assert!(found.is_empty());
}

#[test]
fn prefers_app_sqlite_over_sqlite3_in_repo_data() {
    let repo = tempfile::tempdir().unwrap();
    write(&repo.path().join("config/local.json"), SAFE_CONFIG);
    write_ok_sqlite(&repo.path().join("data/app.sqlite"));
    write_ok_sqlite(&repo.path().join("data/app.sqlite3"));

    let found = detect_legacy_roots(&[LegacySearchRoot::Repository(repo.path().to_path_buf())]);

    assert_eq!(
        found[0].sqlite_path.as_deref(),
        Some(repo.path().join("data/app.sqlite").as_path())
    );
}

#[test]
fn backup_writes_scrubbed_layout_and_hashes() {
    let repo = tempfile::tempdir().unwrap();
    write(&repo.path().join("config/local.json"), SAFE_CONFIG);
    write_ok_sqlite(&repo.path().join("data/app.sqlite"));
    write(
        &repo.path().join("data/materials/resume.md"),
        "工作经历\n2019.03-2021.06 负责订单服务。",
    );
    let root = detect_legacy_roots(&[LegacySearchRoot::Repository(repo.path().to_path_buf())])
        .into_iter()
        .next()
        .unwrap();
    let archive = tempfile::tempdir().unwrap();

    let report = backup_legacy(&root, archive.path()).unwrap();

    assert_eq!(report.archive_path, archive.path());
    assert!(archive.path().join("manifest.json").is_file());
    assert!(archive.path().join("config.json").is_file());
    assert!(archive.path().join("app.sqlite").is_file());
    assert!(archive.path().join("materials/resume.md").is_file());
    assert!(!archive.path().join("app.sqlite3").exists());
    assert!(report.files.contains(&"config.json".into()));
    assert!(report.files.contains(&"app.sqlite".into()));
    assert!(
        report
            .files
            .iter()
            .any(|name| name == "materials/resume.md")
    );
    assert!(report.omitted.is_empty());

    let config_json = fs::read_to_string(archive.path().join("config.json")).unwrap();
    let manifest_json = fs::read_to_string(archive.path().join("manifest.json")).unwrap();
    assert!(
        !contains_secret_material(&config_json),
        "config.json leaked secret material: {config_json}"
    );
    assert!(
        !contains_secret_material(&manifest_json),
        "manifest.json leaked secret material: {manifest_json}"
    );
    let manifest: serde_json::Value = serde_json::from_str(&manifest_json).unwrap();
    assert!(manifest["hashes"]["config.json"].as_str().is_some());
    assert!(manifest["hashes"]["app.sqlite"].as_str().is_some());
    assert!(manifest["hashes"]["materials/resume.md"].as_str().is_some());
    assert_eq!(
        Connection::open_with_flags(
            archive.path().join("app.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap()
        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        .unwrap(),
        "ok"
    );
    allow_cleanup(archive.path());
}

#[test]
fn backup_omits_secret_bearing_config_and_forbidden_sidecar_files() {
    let electron = tempfile::tempdir().unwrap();
    write(
        &electron.path().join("config.json"),
        &serde_json::json!({
            "locale": "zh-CN",
            "control_api_token": "desktop-login-token",
            "desktop_session": "session-cookie",
            "cookies": "sid=abc",
            "apiKey": SECRET_DECOY
        })
        .to_string(),
    );
    write(&electron.path().join(".env"), "OPENAI_API_KEY=sk-env");
    write_bytes(
        &electron.path().join("id.pem"),
        b"-----BEGIN PRIVATE KEY-----\n",
    );
    write(&electron.path().join("Cookies"), "sid=abc");
    write_ok_sqlite(&electron.path().join("app.sqlite"));
    let root = crate::migrate::LegacyRoot {
        kind: LegacyKind::ElectronUserData,
        path: electron.path().to_path_buf(),
        config_path: Some(electron.path().join("config.json")),
        sqlite_path: Some(electron.path().join("app.sqlite")),
        materials_path: None,
    };
    let archive = tempfile::tempdir().unwrap();

    let report = backup_legacy(&root, archive.path()).unwrap();

    assert!(!archive.path().join("config.json").exists());
    assert!(!archive.path().join(".env").exists());
    assert!(!archive.path().join("id.pem").exists());
    assert!(!archive.path().join("Cookies").exists());
    assert!(archive.path().join("app.sqlite").is_file());
    assert!(archive.path().join("manifest.json").is_file());
    for name in ["config.json", ".env", "id.pem", "Cookies"] {
        assert!(
            report.omitted.iter().any(|item| item == name),
            "expected {name} in omitted: {:?}",
            report.omitted
        );
    }
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(archive.path().join("manifest.json")).unwrap())
            .unwrap();
    let omitted = manifest["omitted"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    assert!(omitted.contains(&"config.json".into()));
    assert!(!contains_secret_material(
        &fs::read_to_string(archive.path().join("manifest.json")).unwrap()
    ));
    allow_cleanup(archive.path());
}

#[test]
fn backup_skips_missing_sqlite_and_omits_corrupt_or_account_sqlite() {
    let missing = tempfile::tempdir().unwrap();
    write(&missing.path().join("config/local.json"), SAFE_CONFIG);
    let missing_root =
        detect_legacy_roots(&[LegacySearchRoot::Repository(missing.path().to_path_buf())])
            .into_iter()
            .next()
            .unwrap();
    let missing_archive = tempfile::tempdir().unwrap();
    let skipped = backup_legacy(&missing_root, missing_archive.path()).unwrap();
    assert!(!missing_archive.path().join("app.sqlite").exists());
    assert!(!skipped.files.iter().any(|name| name == "app.sqlite"));
    assert!(!skipped.omitted.iter().any(|name| name == "app.sqlite"));
    assert!(missing_archive.path().join("config.json").is_file());
    allow_cleanup(missing_archive.path());

    let corrupt = tempfile::tempdir().unwrap();
    write(&corrupt.path().join("config/local.json"), SAFE_CONFIG);
    write(
        &corrupt.path().join("data/app.sqlite"),
        "not-a-sqlite-database",
    );
    let corrupt_root =
        detect_legacy_roots(&[LegacySearchRoot::Repository(corrupt.path().to_path_buf())])
            .into_iter()
            .next()
            .unwrap();
    let corrupt_archive = tempfile::tempdir().unwrap();
    let omitted_corrupt = backup_legacy(&corrupt_root, corrupt_archive.path()).unwrap();
    assert!(!corrupt_archive.path().join("app.sqlite").exists());
    assert!(
        omitted_corrupt
            .omitted
            .iter()
            .any(|name| name == "app.sqlite")
    );
    allow_cleanup(corrupt_archive.path());

    let accounts = tempfile::tempdir().unwrap();
    write(&accounts.path().join("config/local.json"), SAFE_CONFIG);
    write_account_sqlite(&accounts.path().join("data/app.sqlite"));
    let accounts_root =
        detect_legacy_roots(&[LegacySearchRoot::Repository(accounts.path().to_path_buf())])
            .into_iter()
            .next()
            .unwrap();
    let accounts_archive = tempfile::tempdir().unwrap();
    let omitted_accounts = backup_legacy(&accounts_root, accounts_archive.path()).unwrap();
    assert!(!accounts_archive.path().join("app.sqlite").exists());
    assert!(
        omitted_accounts
            .omitted
            .iter()
            .any(|name| name == "app.sqlite")
    );
    allow_cleanup(accounts_archive.path());
}

#[test]
fn versioned_backup_is_readonly_under_backups_legacy_stamp() {
    let repo = tempfile::tempdir().unwrap();
    write(&repo.path().join("config/local.json"), SAFE_CONFIG);
    write_ok_sqlite(&repo.path().join("data/app.sqlite"));
    let root = detect_legacy_roots(&[LegacySearchRoot::Repository(repo.path().to_path_buf())])
        .into_iter()
        .next()
        .unwrap();
    let app_data = tempfile::tempdir().unwrap();
    let now = Utc.with_ymd_and_hms(2026, 9, 6, 2, 16, 0).unwrap();

    let report = backup_legacy_into_app_data(&root, app_data.path(), now).unwrap();

    assert_eq!(
        report.archive_path,
        app_data
            .path()
            .join("backups")
            .join("legacy-20260906T021600Z")
    );
    assert!(report.archive_path.join("manifest.json").is_file());
    assert!(
        fs::metadata(report.archive_path.join("manifest.json"))
            .unwrap()
            .permissions()
            .readonly()
    );
    assert!(
        fs::metadata(report.archive_path.join("config.json"))
            .unwrap()
            .permissions()
            .readonly()
    );
    allow_cleanup(app_data.path());
}

fn file_hash(path: &Path) -> String {
    crate::materials::backup::file_sha256(path).unwrap()
}

fn write_legacy_manifest(archive: &Path, hashes: &[(&str, String)], omitted: &[&str]) {
    let mut map = serde_json::Map::new();
    for (key, hash) in hashes {
        map.insert((*key).to_owned(), serde_json::Value::String(hash.clone()));
    }
    write(
        &archive.join("manifest.json"),
        &serde_json::json!({
            "schemaVersion": 1,
            "kind": "legacy",
            "hashes": map,
            "omitted": omitted,
        })
        .to_string(),
    );
}

fn snapshot_tree(root: &Path) -> Vec<(String, Vec<u8>, bool)> {
    let mut entries = Vec::new();
    snapshot_tree_into(root, root, &mut entries);
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}

fn snapshot_tree_into(root: &Path, current: &Path, entries: &mut Vec<(String, Vec<u8>, bool)>) {
    if current.is_file() {
        let relative = current
            .strip_prefix(root)
            .unwrap_or(current)
            .to_string_lossy()
            .replace('\\', "/");
        let readonly = fs::metadata(current).unwrap().permissions().readonly();
        entries.push((relative, fs::read(current).unwrap(), readonly));
        return;
    }
    if !current.is_dir() {
        return;
    }
    let mut children = fs::read_dir(current)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    children.sort();
    for child in children {
        snapshot_tree_into(root, &child, entries);
    }
}

#[test]
fn verify_accepts_readonly_task1_archive_on_temp_copy() {
    let repo = tempfile::tempdir().unwrap();
    write(&repo.path().join("config/local.json"), SAFE_CONFIG);
    write_ok_sqlite(&repo.path().join("data/app.sqlite"));
    write(
        &repo.path().join("data/materials/resume.md"),
        "工作经历\n2019.03-2021.06 负责订单服务。",
    );
    let root = detect_legacy_roots(&[LegacySearchRoot::Repository(repo.path().to_path_buf())])
        .into_iter()
        .next()
        .unwrap();
    let archive = tempfile::tempdir().unwrap();
    let backup = backup_legacy(&root, archive.path()).unwrap();
    let before = snapshot_tree(archive.path());
    let source_before = snapshot_tree(repo.path());

    let verified = verify_legacy_archive(archive.path()).unwrap();

    assert_eq!(verified.archive_path, archive.path());
    assert!(verified.files.contains(&"config.json".into()));
    assert!(verified.files.contains(&"app.sqlite".into()));
    assert!(
        verified
            .files
            .iter()
            .any(|name| name == "materials/resume.md")
    );
    assert_eq!(verified.omitted, backup.omitted);
    assert_eq!(snapshot_tree(archive.path()), before);
    assert_eq!(snapshot_tree(repo.path()), source_before);
    assert!(!archive.path().join("app.sqlite-wal").exists());
    assert!(!archive.path().join("app.sqlite-shm").exists());
    allow_cleanup(archive.path());
}

#[test]
fn verify_allows_missing_config_and_sqlite() {
    let archive = tempfile::tempdir().unwrap();
    write_legacy_manifest(archive.path(), &[], &[]);

    let verified = verify_legacy_archive(archive.path()).unwrap();

    assert!(verified.files.is_empty());
    assert!(verified.omitted.is_empty());
}

#[test]
fn verify_rejects_hash_mismatch_integrity_failure_and_secret_bytes() {
    let mismatched = tempfile::tempdir().unwrap();
    write(&mismatched.path().join("config.json"), SAFE_CONFIG);
    write_legacy_manifest(
        mismatched.path(),
        &[(
            "config.json",
            "0000000000000000000000000000000000000000000000000000000000000000".into(),
        )],
        &[],
    );
    assert_eq!(
        verify_legacy_archive(mismatched.path()).unwrap_err().code(),
        "MIGRATE_HASH_MISMATCH"
    );

    let corrupt = tempfile::tempdir().unwrap();
    write_bytes(&corrupt.path().join("app.sqlite"), b"not-a-sqlite-database");
    write_legacy_manifest(
        corrupt.path(),
        &[("app.sqlite", file_hash(&corrupt.path().join("app.sqlite")))],
        &[],
    );
    assert_eq!(
        verify_legacy_archive(corrupt.path()).unwrap_err().code(),
        "MIGRATE_INTEGRITY_FAILED"
    );

    let secrets = tempfile::tempdir().unwrap();
    write(
        &secrets.path().join("config.json"),
        &format!(r#"{{"note":"{SECRET_DECOY}"}}"#),
    );
    write_legacy_manifest(
        secrets.path(),
        &[(
            "config.json",
            file_hash(&secrets.path().join("config.json")),
        )],
        &[],
    );
    assert_eq!(
        verify_legacy_archive(secrets.path()).unwrap_err().code(),
        "MIGRATE_SECRET_FORBIDDEN"
    );
}

#[test]
fn switch_copies_verified_tree_into_tauri_layout() {
    let repo = tempfile::tempdir().unwrap();
    write(&repo.path().join("config/local.json"), SAFE_CONFIG);
    write_ok_sqlite(&repo.path().join("data/app.sqlite"));
    write(
        &repo.path().join("data/materials/resume.md"),
        "工作经历\n2019.03-2021.06 负责订单服务。",
    );
    let root = detect_legacy_roots(&[LegacySearchRoot::Repository(repo.path().to_path_buf())])
        .into_iter()
        .next()
        .unwrap();
    let archive = tempfile::tempdir().unwrap();
    backup_legacy(&root, archive.path()).unwrap();
    let source_before = snapshot_tree(repo.path());
    let archive_before = snapshot_tree(archive.path());
    let dest = tempfile::tempdir().unwrap();

    let report = switch_legacy_archive(archive.path(), dest.path()).unwrap();

    assert_eq!(report.dest_data_dir, dest.path());
    assert!(report.files.contains(&"config.json".into()));
    assert!(report.files.contains(&"app.sqlite3".into()));
    assert!(
        report
            .files
            .iter()
            .any(|name| name == "materials/resume.md")
    );
    assert!(!report.files.iter().any(|name| name == "app.sqlite"));
    assert!(dest.path().join("config.json").is_file());
    assert!(dest.path().join("app.sqlite3").is_file());
    assert!(!dest.path().join("app.sqlite").exists());
    assert!(dest.path().join("materials/resume.md").is_file());
    assert_eq!(report.marker_path, dest.path().join("migrated-from"));
    let marker_json = fs::read_to_string(&report.marker_path).unwrap();
    assert!(
        !contains_secret_material(&marker_json),
        "migrated-from leaked secret material: {marker_json}"
    );
    let marker: serde_json::Value = serde_json::from_str(&marker_json).unwrap();
    assert_eq!(
        marker["archivePath"].as_str(),
        Some(archive.path().to_string_lossy().as_ref())
    );
    assert!(marker["utc"].as_str().is_some_and(|utc| !utc.is_empty()));
    assert_eq!(marker["omitted"], serde_json::json!([]));
    assert_eq!(
        Connection::open_with_flags(
            dest.path().join("app.sqlite3"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap()
        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        .unwrap(),
        "ok"
    );
    assert_eq!(snapshot_tree(repo.path()), source_before);
    assert_eq!(snapshot_tree(archive.path()), archive_before);
    allow_cleanup(archive.path());
}

#[test]
fn switch_does_not_overwrite_existing_marker_or_live_schema() {
    let repo = tempfile::tempdir().unwrap();
    write(&repo.path().join("config/local.json"), SAFE_CONFIG);
    write_ok_sqlite(&repo.path().join("data/app.sqlite"));
    let root = detect_legacy_roots(&[LegacySearchRoot::Repository(repo.path().to_path_buf())])
        .into_iter()
        .next()
        .unwrap();
    let archive = tempfile::tempdir().unwrap();
    backup_legacy(&root, archive.path()).unwrap();
    let dest = tempfile::tempdir().unwrap();
    let first = switch_legacy_archive(archive.path(), dest.path()).unwrap();
    fs::write(dest.path().join("config.json"), r#"{"touched":true}"#).unwrap();
    let after_first = snapshot_tree(dest.path());

    assert_eq!(
        switch_legacy_archive(archive.path(), dest.path())
            .unwrap_err()
            .code(),
        "MIGRATE_ALREADY_APPLIED"
    );
    assert_eq!(snapshot_tree(dest.path()), after_first);
    assert_eq!(
        fs::read_to_string(dest.path().join("config.json")).unwrap(),
        r#"{"touched":true}"#
    );
    assert_eq!(first.marker_path, dest.path().join("migrated-from"));

    let occupied = tempfile::tempdir().unwrap();
    write_ok_sqlite(&occupied.path().join("app.sqlite3"));
    write(&occupied.path().join("keep.txt"), "untouched");
    assert_eq!(
        switch_legacy_archive(archive.path(), occupied.path())
            .unwrap_err()
            .code(),
        "MIGRATE_ALREADY_APPLIED"
    );
    assert!(!occupied.path().join("migrated-from").exists());
    assert!(!occupied.path().join("config.json").exists());
    assert_eq!(
        fs::read_to_string(occupied.path().join("keep.txt")).unwrap(),
        "untouched"
    );
    allow_cleanup(archive.path());
}

#[test]
fn switch_leaves_dest_unchanged_when_verify_or_copy_fails() {
    let bad = tempfile::tempdir().unwrap();
    write(&bad.path().join("config.json"), SAFE_CONFIG);
    write_legacy_manifest(
        bad.path(),
        &[(
            "config.json",
            "0000000000000000000000000000000000000000000000000000000000000000".into(),
        )],
        &[],
    );
    let dest = tempfile::tempdir().unwrap();
    write(&dest.path().join("keep.txt"), "sentinel");

    assert_eq!(
        switch_legacy_archive(bad.path(), dest.path())
            .unwrap_err()
            .code(),
        "MIGRATE_HASH_MISMATCH"
    );
    assert_eq!(
        fs::read_to_string(dest.path().join("keep.txt")).unwrap(),
        "sentinel"
    );
    assert!(!dest.path().join("migrated-from").exists());
    assert!(!dest.path().join("config.json").exists());
    assert!(!dest.path().join("app.sqlite3").exists());

    let repo = tempfile::tempdir().unwrap();
    write(&repo.path().join("config/local.json"), SAFE_CONFIG);
    let root = detect_legacy_roots(&[LegacySearchRoot::Repository(repo.path().to_path_buf())])
        .into_iter()
        .next()
        .unwrap();
    let archive = tempfile::tempdir().unwrap();
    backup_legacy(&root, archive.path()).unwrap();
    let blocked = tempfile::NamedTempFile::new().unwrap();
    let blocked_path = blocked.path().to_path_buf();
    fs::write(&blocked_path, b"not-a-directory").unwrap();

    assert!(switch_legacy_archive(archive.path(), &blocked_path).is_err());
    assert!(blocked_path.is_file());
    assert_eq!(fs::read(&blocked_path).unwrap(), b"not-a-directory");
    allow_cleanup(archive.path());
}

#[test]
fn first_run_is_noop_without_legacy_roots() {
    let dest = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    let now = Utc.with_ymd_and_hms(2026, 9, 6, 2, 0, 0).unwrap();

    let report = try_first_run_legacy_migration(
        dest.path(),
        &[
            LegacySearchRoot::Repository(empty.path().to_path_buf()),
            LegacySearchRoot::AppData(empty.path().to_path_buf()),
        ],
        dest.path(),
        now,
    )
    .unwrap();

    assert!(report.is_none());
    assert!(!dest.path().join("migrated-from").exists());
    assert!(!dest.path().join("app.sqlite3").exists());
    assert!(!dest.path().join("backups").exists());
}

#[test]
fn first_run_switches_before_live_sqlite_exists() {
    let repo = tempfile::tempdir().unwrap();
    write(&repo.path().join("config/local.json"), SAFE_CONFIG);
    write(
        &repo.path().join("data/materials/resume.md"),
        "工作经历\n2019.03-2021.06 负责订单服务。",
    );
    let dest = tempfile::tempdir().unwrap();
    let now = Utc.with_ymd_and_hms(2026, 9, 6, 2, 1, 0).unwrap();
    assert!(!dest.path().join("app.sqlite3").exists());

    let report = try_first_run_legacy_migration(
        dest.path(),
        &[LegacySearchRoot::Repository(repo.path().to_path_buf())],
        dest.path(),
        now,
    )
    .unwrap()
    .expect("legacy switch should apply");

    assert!(dest.path().join("migrated-from").is_file());
    assert_eq!(report.marker_path, dest.path().join("migrated-from"));
    assert!(!dest.path().join("app.sqlite3").exists());
    assert!(
        dest.path()
            .join("backups/legacy-20260906T020100Z/manifest.json")
            .is_file()
    );
    assert!(!contains_secret_material(
        &fs::read_to_string(dest.path().join("migrated-from")).unwrap()
    ));
}

#[test]
fn first_run_skips_dest_that_already_has_schema() {
    let repo = tempfile::tempdir().unwrap();
    write(&repo.path().join("config/local.json"), SAFE_CONFIG);
    let dest = tempfile::tempdir().unwrap();
    write_ok_sqlite(&dest.path().join("app.sqlite3"));
    let now = Utc.with_ymd_and_hms(2026, 9, 6, 2, 2, 0).unwrap();

    let report = try_first_run_legacy_migration(
        dest.path(),
        &[LegacySearchRoot::Repository(repo.path().to_path_buf())],
        dest.path(),
        now,
    )
    .unwrap();

    assert!(report.is_none());
    assert!(!dest.path().join("migrated-from").exists());
    assert!(!dest.path().join("backups").exists());
}

#[test]
fn first_run_failure_leaves_dest_without_marker() {
    let dest = tempfile::NamedTempFile::new().unwrap();
    let repo = tempfile::tempdir().unwrap();
    write(&repo.path().join("config/local.json"), SAFE_CONFIG);
    let now = Utc.with_ymd_and_hms(2026, 9, 6, 2, 3, 0).unwrap();

    assert!(
        try_first_run_legacy_migration(
            dest.path(),
            &[LegacySearchRoot::Repository(repo.path().to_path_buf())],
            dest.path(),
            now,
        )
        .is_err()
    );
    assert!(dest.path().is_file());
    assert!(!dest.path().join("migrated-from").exists());
}

#[test]
fn status_flags_reenter_when_applied_and_slots_empty() {
    let dest = tempfile::tempdir().unwrap();
    write(
        &dest.path().join("migrated-from"),
        r#"{"archivePath":"C:/tmp/archive","utc":"2026-09-06T00:00:00Z","omitted":[".env"]}"#,
    );

    let empty = legacy_migration_status(dest.path(), false);
    assert!(empty.applied);
    assert!(empty.reenter_secrets);
    assert_eq!(empty.omitted, vec![".env".to_owned()]);

    let filled = legacy_migration_status(dest.path(), true);
    assert!(filled.applied);
    assert!(!filled.reenter_secrets);
    assert_eq!(filled.omitted, vec![".env".to_owned()]);

    let fresh = tempfile::tempdir().unwrap();
    let unused = legacy_migration_status(fresh.path(), false);
    assert!(!unused.applied);
    assert!(!unused.reenter_secrets);
    assert!(unused.omitted.is_empty());
}

#[test]
fn status_and_marker_never_include_secret_material() {
    let dest = tempfile::tempdir().unwrap();
    write(
        &dest.path().join("migrated-from"),
        r#"{"archivePath":"C:/tmp/archive","utc":"2026-09-06T00:00:00Z","omitted":["cookies"]}"#,
    );
    let status = serde_json::to_value(legacy_migration_status(dest.path(), false)).unwrap();
    let encoded = status.to_string();
    assert!(!contains_secret_material(&encoded), "{encoded}");
    assert!(status.get("apiKey").is_none());
    assert!(status.get("apiSecret").is_none());
    assert_eq!(status["omitted"], serde_json::json!(["cookies"]));
}

#[test]
fn secret_slots_configured_reads_provider_and_livekit_flags() {
    let empty = AppConfigV1::from_json(SAFE_CONFIG).unwrap();
    assert!(!secret_slots_configured(&empty));

    let provider = AppConfigV1::from_json(
        r#"{"configVersion":1,"models":{"providers":[{"id":"p1","baseUrl":"https://one.example","credential":{"reference":"providers/p1/api-key","configured":true}}]}}"#,
    )
    .unwrap();
    assert!(secret_slots_configured(&provider));

    let livekit = AppConfigV1::from_json(
        r#"{"configVersion":1,"transport":{"livekit":{"enabled":false,"url":null,"apiKey":{"reference":"transport/livekit/api-key","configured":true},"apiSecret":{"reference":"transport/livekit/api-secret","configured":false},"ready":false,"status":null,"configVersion":0}}}"#,
    )
    .unwrap();
    assert!(secret_slots_configured(&livekit));
}

#[test]
fn locator_roots_are_repo_in_dev_and_roaming_parent_in_release() {
    let dirs = ConfigDirs {
        repository: std::path::PathBuf::from(r"E:\source\assistant"),
        roaming_app_data: std::path::PathBuf::from(r"C:\Users\tester\AppData\Roaming"),
    };
    assert_eq!(
        legacy_search_roots(&dirs, true),
        vec![LegacySearchRoot::Repository(dirs.repository.clone())]
    );
    assert_eq!(
        legacy_search_roots(&dirs, false),
        vec![LegacySearchRoot::AppData(dirs.roaming_app_data.clone())]
    );
}
