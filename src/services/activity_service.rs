/// services/activity_service.rs — Logica di business delle attività
///
/// Elenco paginato con filtri (proprio o di un atleta collegato) ed
/// eliminazione di un'attività con il suo file originale. Non conosce HTTP.
use std::path::Path;

use axum::http::StatusCode;
use sqlx::PgPool;

use crate::dto::activity::{ActivityDto, ActivityPageDto, ListActivitiesQuery};
use crate::errors::AppError;
use crate::middleware::AuthUser;
use crate::parsing::Sport;
use crate::repositories::{activity_repository, user_repository};
use crate::services::access_service;
use crate::storage;

const DEFAULT_PER_PAGE: i64 = 20;
const MAX_PER_PAGE: i64 = 100;

fn validation_error(message: &str) -> AppError {
    AppError::coded(StatusCode::BAD_REQUEST, "VALIDATION_FAILED", message)
}

/// Una pagina di attività dell'utente o di un atleta collegato (`athlete_id`).
///
/// Errori `400 VALIDATION_FAILED`: `page < 1`, `per_page` fuori da 1..=100,
/// `from > to`, sport non valido. Un input errato è 400 per chiunque, prima
/// del controllo d'accesso.
pub async fn list(
    pool: &PgPool,
    auth: &AuthUser,
    query: ListActivitiesQuery,
) -> Result<ActivityPageDto, AppError> {
    let page = query.page.unwrap_or(1);
    let per_page = query.per_page.unwrap_or(DEFAULT_PER_PAGE);
    if page < 1 {
        return Err(validation_error("page deve essere almeno 1"));
    }
    if !(1..=MAX_PER_PAGE).contains(&per_page) {
        return Err(validation_error(
            "per_page deve essere compreso tra 1 e 100",
        ));
    }
    if let (Some(from), Some(to)) = (query.from, query.to)
        && from > to
    {
        return Err(validation_error("from non può essere successivo a to"));
    }
    if let Some(sport) = query.sport.as_deref()
        && Sport::from_str_opt(sport).is_none()
    {
        return Err(validation_error("sport non valido"));
    }

    let target = access_service::resolve_target(pool, auth, query.athlete_id).await?;
    // Le date `from`/`to` sono giorni nel fuso dell'atleta di cui si leggono i dati
    let timezone = user_repository::timezone_of(pool, target).await?;
    let filter = activity_repository::ActivityFilter {
        from: query.from,
        to: query.to,
        sport: query.sport.as_deref(),
        timezone: &timezone,
    };

    let offset = (page - 1).saturating_mul(per_page);
    let rows = activity_repository::list(pool, target, &filter, per_page, offset).await?;
    let total = activity_repository::count(pool, target, &filter).await?;

    Ok(ActivityPageDto {
        items: rows.into_iter().map(ActivityDto::from).collect(),
        total,
        page,
        per_page,
    })
}

/// Elimina un'attività dell'utente e, se non più referenziato, il file originale.
///
/// Solo il proprietario: per chiunque altro (coach compreso) l'attività "non esiste" (404).
/// Prima si elimina la riga, poi il file: un errore sul file viene solo loggato,
/// l'attività è comunque eliminata.
pub async fn delete(
    pool: &PgPool,
    upload_dir: &Path,
    auth: &AuthUser,
    id: i64,
) -> Result<(), AppError> {
    let file_path = activity_repository::delete_owned(pool, id, auth.id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Attività {id} non trovata")))?;

    // Il file si rimuove solo se nessun'altra attività lo referenzia
    remove_file_if_unreferenced(pool, upload_dir, &file_path).await;
    Ok(())
}

/// Rimuove il file originale salvato se nessuna attività lo referenzia.
///
/// Gli errori vengono solo loggati: un file orfano non deve far fallire
/// l'operazione principale.
pub async fn remove_file_if_unreferenced(pool: &PgPool, upload_dir: &Path, file_path: &str) {
    match activity_repository::file_path_in_use(pool, file_path).await {
        Ok(false) => {
            if let Err(e) = storage::remove(upload_dir, file_path).await {
                tracing::warn!("Rimozione del file {file_path} fallita: {e}");
            }
        }
        Ok(true) => {}
        Err(e) => tracing::warn!("Controllo riferimenti al file {file_path} fallito: {e}"),
    }
}
