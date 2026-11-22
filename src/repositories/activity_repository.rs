/// repositories/activity_repository.rs — Query SQL per le attività
///
/// Le funzioni accettano qualsiasi executor (`&PgPool` oppure `&mut *tx`):
/// l'import esegue controllo dei duplicati e inserimento nella stessa transazione.

use axum::http::StatusCode;
use chrono::{DateTime, Duration, Utc};
use sqlx::PgExecutor;

use crate::errors::AppError;
use crate::parsing::{FileFormat, ParsedActivity};

/// Codice dell'errore restituito da `insert` se il file è già stato importato.
pub const DUPLICATE_FILE: &str = "DUPLICATE_FILE";

/// Lock esclusivo per utente, fino alla fine della transazione.
///
/// Serializza controllo dei duplicati e inserimento tra import concorrenti
/// dello stesso utente: due file diversi della stessa attività non possono
/// passare entrambi il controllo `find_overlapping`. La chiave è l'id utente
/// (nessun altro advisory lock è usato nell'applicazione).
pub async fn lock_user_for_import<'e>(
    executor: impl PgExecutor<'e>,
    user_id: i64,
) -> Result<(), AppError> {
    // `query` e non `query!`: la funzione ritorna `void`, tipo non mappato da sqlx
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(user_id)
        .execute(executor)
        .await?;
    Ok(())
}

/// Id dell'attività dell'utente importata da un file con questo SHA-256.
pub async fn find_by_hash<'e>(
    executor: impl PgExecutor<'e>,
    user_id: i64,
    sha: &str,
) -> Result<Option<i64>, AppError> {
    let id = sqlx::query_scalar!(
        "SELECT id FROM activities WHERE user_id = $1 AND file_sha256 = $2",
        user_id,
        sha
    )
    .fetch_optional(executor)
    .await?;

    Ok(id)
}

/// Attività dell'utente dello stesso sport iniziata entro ±`tolerance_s` secondi
/// da `start` (la più vicina, se più d'una).
pub async fn find_overlapping<'e>(
    executor: impl PgExecutor<'e>,
    user_id: i64,
    sport: &str,
    start: DateTime<Utc>,
    tolerance_s: i64,
) -> Result<Option<i64>, AppError> {
    let tolerance = Duration::seconds(tolerance_s);
    let id = sqlx::query_scalar!(
        r#"
        SELECT id FROM activities
        WHERE user_id = $1 AND sport = $2
          AND start_time BETWEEN $3 AND $4
        ORDER BY ABS(EXTRACT(EPOCH FROM start_time - $5)), id
        LIMIT 1
        "#,
        user_id,
        sport,
        start - tolerance,
        start + tolerance,
        start
    )
    .fetch_optional(executor)
    .await?;

    Ok(id)
}

/// Inserisce un'attività importata da file e ne ritorna l'id.
///
/// Se l'utente ha già un'attività con lo stesso SHA-256 (import concorrente
/// dello stesso file) il vincolo `activities_user_sha_unique` fallisce →
/// `AppError::Coded` con codice `DUPLICATE_FILE` (`409`).
pub async fn insert<'e>(
    executor: impl PgExecutor<'e>,
    user_id: i64,
    a: &ParsedActivity,
    format: FileFormat,
    file_path: &str,
    sha: &str,
) -> Result<i64, AppError> {
    let result = sqlx::query_scalar!(
        r#"
        INSERT INTO activities (
            user_id, sport, sport_raw, name, start_time, utc_offset_seconds,
            elapsed_s, moving_s, distance_m, elevation_gain_m, avg_hr, max_hr,
            avg_speed_mps, avg_power_w, avg_cadence, calories,
            file_format, file_path, file_sha256
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19)
        RETURNING id
        "#,
        user_id,
        a.sport.as_str(),
        a.sport_raw,
        a.name,
        a.start_time,
        a.utc_offset_seconds,
        a.elapsed_s,
        a.moving_s,
        a.distance_m,
        a.elevation_gain_m,
        a.avg_hr,
        a.max_hr,
        a.avg_speed_mps,
        a.avg_power_w,
        a.avg_cadence,
        a.calories,
        format.as_str(),
        file_path,
        sha
    )
    .fetch_one(executor)
    .await;

    match result {
        Ok(id) => Ok(id),
        Err(sqlx::Error::Database(db)) if db.constraint() == Some("activities_user_sha_unique") => {
            Err(AppError::coded(
                StatusCode::CONFLICT,
                DUPLICATE_FILE,
                "File già importato",
            ))
        }
        Err(e) => Err(AppError::Database(e)),
    }
}
