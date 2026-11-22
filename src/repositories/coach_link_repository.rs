/// repositories/coach_link_repository.rs — Query SQL per i link allenatore-atleta
///
/// Solo SQL, nessuna logica di business. Le transizioni di stato sono
/// UPDATE condizionali (`WHERE status = <atteso>`): controllo e scrittura
/// avvengono in un'unica istruzione atomica, quindi due richieste
/// concorrenti sullo stesso link non possono avere entrambe successo.
use axum::http::StatusCode;
use sqlx::PgPool;

use crate::errors::AppError;
use crate::models::coach_link::{CoachLink, LinkStatus, LinkedAthlete};

/// Crea un link `pending` e ne ritorna l'id.
///
/// Se l'atleta ha già un link aperto (pending/active) l'indice univoco parziale
/// `coach_links_one_open` fallisce → `409 COACH_LINK_EXISTS`. Affidarsi
/// all'indice (e non a un controllo preventivo) copre anche le richieste concorrenti.
pub async fn create(pool: &PgPool, athlete_id: i64, coach_id: i64) -> Result<i64, AppError> {
    let result = sqlx::query_scalar!(
        r#"
        INSERT INTO coach_links (athlete_id, coach_id, status)
        VALUES ($1, $2, 'pending')
        RETURNING id
        "#,
        athlete_id,
        coach_id
    )
    .fetch_one(pool)
    .await;

    match result {
        Ok(id) => Ok(id),
        Err(sqlx::Error::Database(db)) if db.constraint() == Some("coach_links_one_open") => {
            Err(AppError::coded(
                StatusCode::CONFLICT,
                "COACH_LINK_EXISTS",
                "Esiste già una richiesta o un coach attivo",
            ))
        }
        Err(e) => Err(AppError::Database(e)),
    }
}

/// Legge un link per id (con gli username delle parti).
pub async fn find_by_id(pool: &PgPool, id: i64) -> Result<Option<CoachLink>, AppError> {
    let link = sqlx::query_as!(
        CoachLink,
        r#"
        SELECT cl.id, cl.athlete_id, a.username AS athlete_username,
               cl.coach_id, c.username AS coach_username,
               cl.status, cl.requested_at, cl.responded_at, cl.ended_at
        FROM coach_links cl
        JOIN users a ON a.id = cl.athlete_id
        JOIN users c ON c.id = cl.coach_id
        WHERE cl.id = $1
        "#,
        id
    )
    .fetch_optional(pool)
    .await?;

    Ok(link)
}

/// Link dell'atleta (`as_coach = false`) o diretti al coach (`as_coach = true`),
/// opzionalmente filtrati per stato, dal più recente.
pub async fn list_for_user(
    pool: &PgPool,
    user_id: i64,
    as_coach: bool,
    status: Option<LinkStatus>,
) -> Result<Vec<CoachLink>, AppError> {
    let links = sqlx::query_as!(
        CoachLink,
        r#"
        SELECT cl.id, cl.athlete_id, a.username AS athlete_username,
               cl.coach_id, c.username AS coach_username,
               cl.status, cl.requested_at, cl.responded_at, cl.ended_at
        FROM coach_links cl
        JOIN users a ON a.id = cl.athlete_id
        JOIN users c ON c.id = cl.coach_id
        WHERE (CASE WHEN $2 THEN cl.coach_id ELSE cl.athlete_id END) = $1
          AND ($3::text IS NULL OR cl.status = $3)
        ORDER BY cl.requested_at DESC, cl.id DESC
        "#,
        user_id,
        as_coach,
        status.map(|s| s.as_str())
    )
    .fetch_all(pool)
    .await?;

    Ok(links)
}

/// Transizione di stato atomica `from` → `to`.
///
/// `is_response` = true per accept/reject (valorizza `responded_at`),
/// false per cancel/revoke (valorizza `ended_at`).
/// Ritorna `false` se il link non era nello stato `from` (nessuna riga modificata).
pub async fn transition(
    pool: &PgPool,
    id: i64,
    from: LinkStatus,
    to: LinkStatus,
    is_response: bool,
) -> Result<bool, AppError> {
    let result = sqlx::query!(
        r#"
        UPDATE coach_links
        SET status       = $3,
            responded_at = CASE WHEN $4 THEN NOW() ELSE responded_at END,
            ended_at     = CASE WHEN $4 THEN ended_at ELSE NOW() END
        WHERE id = $1 AND status = $2
        "#,
        id,
        from.as_str(),
        to.as_str(),
        is_response
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected() == 1)
}

/// `true` se il coach ha un link `active` verso l'atleta.
pub async fn has_active_link(
    pool: &PgPool,
    coach_id: i64,
    athlete_id: i64,
) -> Result<bool, AppError> {
    let exists = sqlx::query_scalar!(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM coach_links
            WHERE coach_id = $1 AND athlete_id = $2 AND status = 'active'
        ) AS "exists!"
        "#,
        coach_id,
        athlete_id
    )
    .fetch_one(pool)
    .await?;

    Ok(exists)
}

/// Atleti con link `active` verso il coach, in ordine alfabetico di username.
pub async fn list_active_athletes(
    pool: &PgPool,
    coach_id: i64,
) -> Result<Vec<LinkedAthlete>, AppError> {
    let athletes = sqlx::query_as!(
        LinkedAthlete,
        r#"
        SELECT a.id, a.username,
               COALESCE(cl.responded_at, cl.requested_at) AS "linked_since!"
        FROM coach_links cl
        JOIN users a ON a.id = cl.athlete_id
        WHERE cl.coach_id = $1 AND cl.status = 'active'
        ORDER BY a.username
        "#,
        coach_id
    )
    .fetch_all(pool)
    .await?;

    Ok(athletes)
}
