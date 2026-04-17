/// services/user_service.rs — Logica di business per la gestione utenti
///
/// Questo service orchestra le operazioni sugli utenti:
/// - Recupero con paginazione
/// - Aggiornamento (con re-hash della password se cambia)
/// - Eliminazione con verifica esistenza
///
/// Principio: il service è l'unico posto dove si prende la "decisione".
/// Il repository esegue solo SQL, il controller gestisce solo HTTP.

use sqlx::PgPool;
use uuid::Uuid;

use crate::dto::user::{PaginationQuery, UpdateUserRequest, UserListResponse, UserResponse};
use crate::errors::AppError;
use crate::models::user::User;
use crate::repositories::user_repository;

/// Converte un `User` (model DB) in `UserResponse` (DTO pubblico).
/// Funzione privata usata internamente dal service.
/// In questo modo non esponiamo mai `password_hash` per errore.
fn to_response(user: User) -> UserResponse {
    UserResponse {
        id: user.id,
        email: user.email,
        username: user.username,
        is_active: user.is_active,
        created_at: user.created_at,
        updated_at: user.updated_at,
    }
}

/// Recupera la lista paginata di tutti gli utenti.
///
/// Usa i valori di default se `page` o `per_page` non sono forniti nel query param.
/// Restituisce anche il totale per permettere al client di calcolare le pagine.
pub async fn get_all_users(
    pool: &PgPool,
    query: PaginationQuery,
) -> Result<UserListResponse, AppError> {
    let page = query.page.unwrap_or(1).max(1); // minimo pagina 1
    let per_page = query.per_page.unwrap_or(20).clamp(1, 100); // tra 1 e 100

    // Esegui entrambe le query in parallelo con tokio::join!
    // Più efficiente che eseguirle in sequenza
    let (users, total) = tokio::try_join!(
        user_repository::find_all(pool, page, per_page),
        user_repository::count(pool)
    )?;

    Ok(UserListResponse {
        users: users.into_iter().map(to_response).collect(),
        total,
        page,
        per_page,
    })
}

/// Recupera un singolo utente per ID.
/// Restituisce 404 se non esiste.
pub async fn get_user_by_id(pool: &PgPool, id: Uuid) -> Result<UserResponse, AppError> {
    let user = user_repository::find_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Utente con id '{}' non trovato", id)))?;

    Ok(to_response(user))
}

/// Aggiorna i dati di un utente (PATCH — aggiornamento parziale).
///
/// Flusso:
/// 1. Verifica che l'utente esista
/// 2. Se la password cambia, la re-hasha
/// 3. Se l'email cambia, controlla che non sia già in uso
/// 4. Aggiorna via repository
pub async fn update_user(
    pool: &PgPool,
    id: Uuid,
    req: UpdateUserRequest,
) -> Result<UserResponse, AppError> {
    // 1. Verifica esistenza
    user_repository::find_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Utente con id '{}' non trovato", id)))?;

    // 2. Se l'email cambia, verifica che non sia già usata da un altro utente
    if let Some(ref new_email) = req.email {
        if let Some(existing) = user_repository::find_by_email(pool, new_email).await? {
            if existing.id != id {
                return Err(AppError::Conflict(format!(
                    "Email '{}' già in uso",
                    new_email
                )));
            }
        }
    }

    // 3. Se la password cambia, re-hasha (CPU-intensive → spawn_blocking)
    let new_password_hash = if let Some(new_password) = req.password {
        let hash = tokio::task::spawn_blocking(move || {
            bcrypt::hash(new_password, bcrypt::DEFAULT_COST)
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
        Some(hash)
    } else {
        None
    };

    // 4. Aggiorna nel DB
    let updated_user = user_repository::update(
        pool,
        id,
        req.email.as_deref(),        // Option<String> → Option<&str>
        req.username.as_deref(),
        new_password_hash.as_deref(),
    )
    .await?;

    Ok(to_response(updated_user))
}

/// Elimina un utente per ID.
/// Restituisce 404 se non esisteva.
pub async fn delete_user(pool: &PgPool, id: Uuid) -> Result<(), AppError> {
    let rows_affected = user_repository::delete(pool, id).await?;

    if rows_affected == 0 {
        return Err(AppError::NotFound(format!(
            "Utente con id '{}' non trovato",
            id
        )));
    }

    Ok(())
}
