/// controllers/stats_controller.rs — Handler HTTP delle statistiche
///
/// Endpoint protetto da JWT:
///   GET /stats/summary → totali, per sport e serie per periodo (proprio o di un atleta collegato)
use axum::{Json, extract::State};

use crate::AppState;
use crate::dto::stats::{StatsQuery, StatsSummaryDto};
use crate::errors::AppError;
use crate::extractors::AppQuery;
use crate::middleware::AuthUser;
use crate::services::stats_service;

/// GET /stats/summary?from=&to=&sport=&granularity=&athlete_id=
pub async fn summary(
    State(state): State<AppState>,
    auth: AuthUser,
    AppQuery(query): AppQuery<StatsQuery>,
) -> Result<Json<StatsSummaryDto>, AppError> {
    Ok(Json(
        stats_service::get_summary(&state.pool, &auth, query).await?,
    ))
}
