/// controllers/stats_controller.rs — Handler HTTP delle statistiche
///
/// Endpoint protetto da JWT:
///   GET /stats/summary → totali, per sport e serie per periodo (proprio o di un atleta collegato)

use axum::{extract::State, Json};

use crate::dto::stats::{StatsQuery, StatsSummaryDto};
use crate::errors::AppError;
use crate::extractors::AppQuery;
use crate::middleware::AuthUser;
use crate::repositories::user_repository;
use crate::services::{access_service, stats_service};
use crate::AppState;

/// GET /stats/summary?from=&to=&sport=&granularity=&athlete_id=
pub async fn summary(
    State(state): State<AppState>,
    auth: AuthUser,
    AppQuery(query): AppQuery<StatsQuery>,
) -> Result<Json<StatsSummaryDto>, AppError> {
    // Un input errato è 400 per chiunque, prima del controllo d'accesso
    let request = stats_service::validate(query)?;

    let target = access_service::resolve_target(&state.pool, &auth, request.athlete_id).await?;
    // Giorni e periodi sono calcolati nel fuso dell'atleta di cui si leggono i dati
    let timezone = user_repository::timezone_of(&state.pool, target).await?;

    let summary = stats_service::summary(&state.pool, target, &timezone, &request).await?;
    Ok(Json(summary))
}
