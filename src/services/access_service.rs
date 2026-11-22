/// services/access_service.rs — Controllo d'accesso ai dati di un atleta
///
/// UNICO punto in cui si decide se un utente può vedere i dati di un altro:
/// ogni endpoint che accetta un `athlete_id` opzionale deve passare da
/// `resolve_target`. Un coach vede i dati di un atleta solo con un link
/// `active`; ogni altro caso → `403 ATHLETE_ACCESS_DENIED`.

use axum::http::StatusCode;
use sqlx::PgPool;

use crate::errors::AppError;
use crate::middleware::AuthUser;
use crate::models::user::Role;
use crate::repositories::coach_link_repository;

/// Risolve l'id dell'utente i cui dati vanno letti.
///
/// - `None` oppure l'id del viewer → il viewer stesso;
/// - altrimenti il viewer deve essere un `coach` con link `active` verso
///   quell'atleta. Un atleta che chiede i dati di un altro, un coach senza
///   link (o con link pending/rejected/cancelled/revoked) e un atleta
///   inesistente ricevono lo stesso errore: nessuna differenza osservabile.
pub async fn resolve_target(
    pool: &PgPool,
    viewer: &AuthUser,
    athlete_id: Option<i64>,
) -> Result<i64, AppError> {
    let target = match athlete_id {
        None => return Ok(viewer.id),
        Some(id) if id == viewer.id => return Ok(viewer.id),
        Some(id) => id,
    };

    if viewer.role == Role::Coach
        && coach_link_repository::has_active_link(pool, viewer.id, target).await?
    {
        return Ok(target);
    }

    Err(AppError::coded(
        StatusCode::FORBIDDEN,
        "ATHLETE_ACCESS_DENIED",
        "Non hai accesso ai dati di questo atleta",
    ))
}
