/// repositories/import_repository.rs — Query SQL per gli import e i loro item
use axum::http::StatusCode;
use sqlx::PgPool;

use crate::errors::AppError;
use crate::models::import::{Import, ImportItem, ImportStatus, ItemStatus};

/// Codice dell'errore restituito da `create` se l'utente ha già un import in corso.
pub const IMPORT_IN_PROGRESS: &str = "IMPORT_IN_PROGRESS";

/// Crea un import `processing` per l'utente.
///
/// Se l'utente ha già un import `processing` l'indice unico parziale
/// `imports_one_processing_per_user` fallisce → `409 IMPORT_IN_PROGRESS`.
pub async fn create(pool: &PgPool, user_id: i64) -> Result<Import, AppError> {
    let import = sqlx::query_as!(
        Import,
        r#"
        INSERT INTO imports (user_id, status)
        VALUES ($1, $2)
        RETURNING id, user_id, status, created_at, finished_at
        "#,
        user_id,
        ImportStatus::Processing.as_str()
    )
    .fetch_one(pool)
    .await;

    match import {
        Ok(import) => Ok(import),
        Err(sqlx::Error::Database(db))
            if db.constraint() == Some("imports_one_processing_per_user") =>
        {
            Err(AppError::coded(
                StatusCode::CONFLICT,
                IMPORT_IN_PROGRESS,
                "Un import è già in corso: attendi che termini",
            ))
        }
        Err(e) => Err(AppError::Database(e)),
    }
}

/// `true` se l'utente ha un import `processing`.
pub async fn has_processing(pool: &PgPool, user_id: i64) -> Result<bool, AppError> {
    let found = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM imports WHERE user_id = $1 AND status = $2) AS "found!""#,
        user_id,
        ImportStatus::Processing.as_str()
    )
    .fetch_one(pool)
    .await?;

    Ok(found)
}

/// Import per id, solo se appartiene all'utente.
pub async fn find_for_user(
    pool: &PgPool,
    id: i64,
    user_id: i64,
) -> Result<Option<Import>, AppError> {
    let import = sqlx::query_as!(
        Import,
        r#"
        SELECT id, user_id, status, created_at, finished_at
        FROM imports
        WHERE id = $1 AND user_id = $2
        "#,
        id,
        user_id
    )
    .fetch_optional(pool)
    .await?;

    Ok(import)
}

/// Ultimi `limit` import dell'utente, dal più recente.
pub async fn list_recent(pool: &PgPool, user_id: i64, limit: i64) -> Result<Vec<Import>, AppError> {
    let imports = sqlx::query_as!(
        Import,
        r#"
        SELECT id, user_id, status, created_at, finished_at
        FROM imports
        WHERE user_id = $1
        ORDER BY created_at DESC, id DESC
        LIMIT $2
        "#,
        user_id,
        limit
    )
    .fetch_all(pool)
    .await?;

    Ok(imports)
}

/// Item dell'import, nell'ordine di elaborazione.
pub async fn list_items(pool: &PgPool, import_id: i64) -> Result<Vec<ImportItem>, AppError> {
    let items = sqlx::query_as!(
        ImportItem,
        r#"
        SELECT id, import_id, filename, status, activity_id, error_code
        FROM import_items
        WHERE import_id = $1
        ORDER BY id
        "#,
        import_id
    )
    .fetch_all(pool)
    .await?;

    Ok(items)
}

/// Aggiunge un item all'import e ne ritorna l'id.
pub async fn insert_item(
    pool: &PgPool,
    import_id: i64,
    filename: &str,
    status: ItemStatus,
    error_code: Option<&str>,
) -> Result<i64, AppError> {
    let id = sqlx::query_scalar!(
        r#"
        INSERT INTO import_items (import_id, filename, status, error_code)
        VALUES ($1, $2, $3, $4)
        RETURNING id
        "#,
        import_id,
        filename,
        status.as_str(),
        error_code
    )
    .fetch_one(pool)
    .await?;

    Ok(id)
}

/// Registra l'esito finale di un item.
pub async fn finish_item(
    pool: &PgPool,
    item_id: i64,
    status: ItemStatus,
    activity_id: Option<i64>,
    error_code: Option<&str>,
) -> Result<(), AppError> {
    sqlx::query!(
        r#"
        UPDATE import_items
        SET status = $2, activity_id = $3, error_code = $4
        WHERE id = $1
        "#,
        item_id,
        status.as_str(),
        activity_id,
        error_code
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Chiude l'import come `completed` e valorizza `finished_at`.
///
/// Nella stessa transazione gli item ancora `pending` di questo import (esito
/// non registrato per un errore) diventano `failed`/`INTERNAL`: un import
/// `completed` non ha mai item `pending`.
pub async fn complete(pool: &PgPool, import_id: i64) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;

    sqlx::query!(
        r#"
        UPDATE import_items
        SET status = $2, error_code = 'INTERNAL'
        WHERE import_id = $1 AND status = $3
        "#,
        import_id,
        ItemStatus::Failed.as_str(),
        ItemStatus::Pending.as_str()
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!(
        r#"
        UPDATE imports
        SET status = $2, finished_at = NOW()
        WHERE id = $1
        "#,
        import_id,
        ImportStatus::Completed.as_str()
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Chiude gli import rimasti a metà per un arresto del server.
///
/// Da chiamare all'avvio, prima di accettare richieste: import `processing`
/// → `failed` (con `finished_at`), item `pending` → `failed` con codice
/// `IMPORT_INTERRUPTED`. Ritorna il numero di import chiusi.
pub async fn fail_interrupted(pool: &PgPool) -> Result<u64, AppError> {
    let mut tx = pool.begin().await?;

    sqlx::query!(
        r#"
        UPDATE import_items
        SET status = $1, error_code = 'IMPORT_INTERRUPTED'
        WHERE status = $2
        "#,
        ItemStatus::Failed.as_str(),
        ItemStatus::Pending.as_str()
    )
    .execute(&mut *tx)
    .await?;

    let result = sqlx::query!(
        r#"
        UPDATE imports
        SET status = $1, finished_at = NOW()
        WHERE status = $2
        "#,
        ImportStatus::Failed.as_str(),
        ImportStatus::Processing.as_str()
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(result.rows_affected())
}
