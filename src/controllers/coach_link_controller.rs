/// controllers/coach_link_controller.rs — Handler HTTP per i link allenatore-atleta
///
/// Endpoint protetti da JWT (la logica di ruolo e di proprietà è nel service):
///   POST /coach-links              → l'atleta richiede un coach (201)
///   GET  /coach-links?status=      → i propri link (atleta) o quelli ricevuti (coach)
///   POST /coach-links/:id/accept   → il coach accetta
///   POST /coach-links/:id/reject   → il coach rifiuta
///   POST /coach-links/:id/cancel   → l'atleta annulla una richiesta pending
///   POST /coach-links/:id/revoke   → l'atleta revoca un coach attivo
///   GET  /athletes                 → gli atleti collegati al coach

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};

use crate::dto::coach_link::{
    CoachLinkDto, CreateCoachLinkRequest, LinkedAthleteDto, ListCoachLinksQuery,
};
use crate::errors::AppError;
use crate::extractors::AppJson;
use crate::middleware::AuthUser;
use crate::services::coach_link_service::{self, LinkAction};
use crate::AppState;

/// POST /coach-links
///
/// Esempio: {"coach": "mario" } oppure {"coach": "mario@example.com"}
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    AppJson(req): AppJson<CreateCoachLinkRequest>,
) -> Result<(StatusCode, Json<CoachLinkDto>), AppError> {
    let link = coach_link_service::request_coach(&state.pool, &auth, &req.coach).await?;
    Ok((StatusCode::CREATED, Json(link)))
}

/// GET /coach-links?status=<opzionale>
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ListCoachLinksQuery>,
) -> Result<Json<Vec<CoachLinkDto>>, AppError> {
    let links = coach_link_service::list_links(&state.pool, &auth, query.status.as_deref()).await?;
    Ok(Json(links))
}

/// POST /coach-links/:id/accept
pub async fn accept(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<CoachLinkDto>, AppError> {
    act(state, auth, id, LinkAction::Accept).await
}

/// POST /coach-links/:id/reject
pub async fn reject(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<CoachLinkDto>, AppError> {
    act(state, auth, id, LinkAction::Reject).await
}

/// POST /coach-links/:id/cancel
pub async fn cancel(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<CoachLinkDto>, AppError> {
    act(state, auth, id, LinkAction::Cancel).await
}

/// POST /coach-links/:id/revoke
pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<CoachLinkDto>, AppError> {
    act(state, auth, id, LinkAction::Revoke).await
}

async fn act(
    state: AppState,
    auth: AuthUser,
    id: i64,
    action: LinkAction,
) -> Result<Json<CoachLinkDto>, AppError> {
    let link = coach_link_service::act_on_link(&state.pool, &auth, id, action).await?;
    Ok(Json(link))
}

/// GET /athletes — atleti con link attivo verso il coach autenticato
pub async fn list_athletes(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<LinkedAthleteDto>>, AppError> {
    let athletes = coach_link_service::list_athletes(&state.pool, &auth).await?;
    Ok(Json(athletes))
}
