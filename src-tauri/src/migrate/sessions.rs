use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::Deserialize;
use ts_rs::TS;

use crate::database::Database;

use super::MigrateError;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct LegacySessionImport {
    pub sessions: u32,
    pub turns: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacySessionPayload {
    #[serde(default)]
    session_id: String,
    status: String,
    #[serde(default)]
    assistant_role: Option<String>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    finished_at: Option<String>,
    #[serde(default)]
    transcript: Vec<LegacyTranscriptItem>,
}

#[derive(Debug, Deserialize)]
struct LegacyTranscriptItem {
    role: String,
    text: String,
    #[serde(default)]
    at: String,
}

struct LegacyRow {
    finished_at: Option<String>,
    payload: String,
    updated_at: String,
}

struct ImportedSession {
    id: String,
    status: String,
    role_profile_id: String,
    started_at: Option<String>,
    finished_at: Option<String>,
    updated_at: String,
    turns: Vec<ImportedTurn>,
}

struct ImportedTurn {
    user_text: String,
    assistant_text: String,
    created_at: String,
}

pub(crate) fn complete_legacy_session_import(dest_data_dir: &Path) -> Result<(), MigrateError> {
    let sqlite = dest_data_dir.join("app.sqlite3");
    if !sqlite.is_file() || !has_legacy_session_tables(&sqlite)? {
        return Ok(());
    }
    let database = Database::open(&sqlite).map_err(|_| MigrateError::Operation)?;
    database.migrate().map_err(|_| MigrateError::Operation)?;
    import_legacy_sessions(&database)?;
    Ok(())
}

pub fn import_legacy_sessions_from_sqlite(
    source: &Path,
    dest: &Database,
) -> Result<LegacySessionImport, MigrateError> {
    insert_imported_sessions(dest, &read_legacy_sessions_from_path(source)?)
}

pub fn import_legacy_sessions(database: &Database) -> Result<LegacySessionImport, MigrateError> {
    insert_imported_sessions(database, &read_legacy_sessions_from_database(database)?)
}

fn has_legacy_session_tables(path: &Path) -> Result<bool, MigrateError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| MigrateError::Operation)?;
    Ok(table_exists(&connection, "current_session")?
        || table_exists(&connection, "archived_sessions")?)
}

fn table_exists(connection: &Connection, name: &str) -> Result<bool, MigrateError> {
    connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![name],
            |_| Ok(true),
        )
        .optional()
        .map(|row| row.unwrap_or(false))
        .map_err(|_| MigrateError::Operation)
}

fn read_legacy_sessions_from_path(path: &Path) -> Result<Vec<ImportedSession>, MigrateError> {
    if !has_legacy_session_tables(path)? {
        return Ok(Vec::new());
    }
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| MigrateError::Operation)?;
    parse_legacy_rows(collect_legacy_rows(&connection).map_err(|_| MigrateError::Operation)?)
}

fn read_legacy_sessions_from_database(
    database: &Database,
) -> Result<Vec<ImportedSession>, MigrateError> {
    let rows = database
        .with_connection(collect_legacy_rows)
        .map_err(|_| MigrateError::Operation)?;
    parse_legacy_rows(rows)
}

fn collect_legacy_rows(connection: &Connection) -> rusqlite::Result<Vec<LegacyRow>> {
    let mut rows = Vec::new();
    let has_archived = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'archived_sessions'",
            [],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if has_archived {
        let mut statement =
            connection.prepare("SELECT finished_at, payload, updated_at FROM archived_sessions")?;
        let mapped = statement.query_map([], |row| {
            Ok(LegacyRow {
                finished_at: row.get(0)?,
                payload: row.get(1)?,
                updated_at: row.get(2)?,
            })
        })?;
        for row in mapped {
            rows.push(row?);
        }
    }
    let has_current = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'current_session'",
            [],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if has_current
        && let Some(row) = connection
            .query_row(
                "SELECT payload, updated_at FROM current_session WHERE singleton_id = 1",
                [],
                |row| {
                    Ok(LegacyRow {
                        finished_at: None,
                        payload: row.get(0)?,
                        updated_at: row.get(1)?,
                    })
                },
            )
            .optional()?
    {
        rows.push(row);
    }
    Ok(rows)
}

fn parse_legacy_rows(rows: Vec<LegacyRow>) -> Result<Vec<ImportedSession>, MigrateError> {
    let mut by_id = BTreeMap::new();
    for row in rows {
        if let Some(session) =
            parse_legacy_session(&row.payload, row.finished_at.as_deref(), &row.updated_at)?
        {
            by_id.entry(session.id.clone()).or_insert(session);
        }
    }
    Ok(by_id.into_values().collect())
}

fn parse_legacy_session(
    payload: &str,
    finished_at_fallback: Option<&str>,
    updated_at: &str,
) -> Result<Option<ImportedSession>, MigrateError> {
    let parsed: LegacySessionPayload =
        serde_json::from_str(payload).map_err(|_| MigrateError::PayloadInvalid)?;
    if parsed.session_id.trim().is_empty() {
        return Ok(None);
    }
    let started_at = empty_to_none(parsed.started_at);
    let finished_at =
        empty_to_none(parsed.finished_at).or_else(|| finished_at_fallback.map(ToOwned::to_owned));
    let updated = finished_at
        .clone()
        .or_else(|| started_at.clone())
        .unwrap_or_else(|| updated_at.to_owned());
    Ok(Some(ImportedSession {
        id: parsed.session_id,
        status: map_status(&parsed.status)?,
        role_profile_id: parsed
            .assistant_role
            .filter(|role| !role.is_empty())
            .unwrap_or_else(|| "legacy".into()),
        started_at,
        finished_at,
        updated_at: updated,
        turns: pair_transcript(&parsed.transcript)?,
    }))
}

fn map_status(status: &str) -> Result<String, MigrateError> {
    match status {
        "finished" => Ok("completed".into()),
        "running" | "idle" => Ok("interrupted".into()),
        _ => Err(MigrateError::PayloadInvalid),
    }
}

fn empty_to_none(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let trimmed = text.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

fn pair_transcript(items: &[LegacyTranscriptItem]) -> Result<Vec<ImportedTurn>, MigrateError> {
    let mut turns = Vec::new();
    let mut pending_user: Option<&LegacyTranscriptItem> = None;
    let mut pending_assistant: Option<&LegacyTranscriptItem> = None;
    for item in items {
        match item.role.as_str() {
            "candidate" => {
                if let Some(user) = pending_user.take() {
                    turns.push(imported_turn(user, pending_assistant.take()));
                }
                pending_user = Some(item);
                if pending_assistant.is_some()
                    && let Some(user) = pending_user.take()
                {
                    turns.push(imported_turn(user, pending_assistant.take()));
                }
            }
            "interviewer" => {
                pending_assistant = Some(item);
                if let Some(user) = pending_user.take() {
                    turns.push(imported_turn(user, pending_assistant.take()));
                }
            }
            _ => return Err(MigrateError::PayloadInvalid),
        }
    }
    if let Some(user) = pending_user {
        turns.push(imported_turn(user, pending_assistant));
    } else if let Some(assistant) = pending_assistant {
        turns.push(ImportedTurn {
            user_text: String::new(),
            assistant_text: assistant.text.clone(),
            created_at: timestamp_or_default(&assistant.at),
        });
    }
    Ok(turns)
}

fn imported_turn(
    user: &LegacyTranscriptItem,
    assistant: Option<&LegacyTranscriptItem>,
) -> ImportedTurn {
    let created_at = if user.at.is_empty() {
        timestamp_or_default(assistant.map(|item| item.at.as_str()).unwrap_or(""))
    } else {
        timestamp_or_default(&user.at)
    };
    ImportedTurn {
        user_text: user.text.clone(),
        assistant_text: assistant.map(|item| item.text.clone()).unwrap_or_default(),
        created_at,
    }
}

fn timestamp_or_default(value: &str) -> String {
    if value.trim().is_empty() {
        "1970-01-01T00:00:00Z".into()
    } else {
        value.to_owned()
    }
}

fn insert_imported_sessions(
    database: &Database,
    sessions: &[ImportedSession],
) -> Result<LegacySessionImport, MigrateError> {
    database
        .with_transaction(|transaction| {
            let mut imported_sessions = 0_u32;
            let mut imported_turns = 0_u32;
            for session in sessions {
                let exists: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?1)",
                    params![session.id],
                    |row| row.get(0),
                )?;
                if exists {
                    continue;
                }
                transaction.execute(
                    "INSERT INTO sessions(
                        id, status, role_profile_id, voice_route_id, transport_mode,
                        started_at, finished_at, updated_at
                     ) VALUES (?1, ?2, ?3, '', 'direct', ?4, ?5, ?6)",
                    params![
                        session.id,
                        session.status,
                        session.role_profile_id,
                        session.started_at,
                        session.finished_at,
                        session.updated_at,
                    ],
                )?;
                imported_sessions += 1;
                for (index, turn) in session.turns.iter().enumerate() {
                    transaction.execute(
                        "INSERT INTO session_turns(
                            id, session_id, turn_index, user_text, assistant_text,
                            materials_used, created_at
                         ) VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
                        params![
                            format!("{}-t{index}", session.id),
                            session.id,
                            index as i64,
                            turn.user_text,
                            turn.assistant_text,
                            turn.created_at,
                        ],
                    )?;
                    imported_turns += 1;
                }
            }
            Ok(LegacySessionImport {
                sessions: imported_sessions,
                turns: imported_turns,
            })
        })
        .map_err(|_| MigrateError::Operation)
}
