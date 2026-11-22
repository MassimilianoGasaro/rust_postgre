/// services/stats_service.rs — Statistiche per intervallo di date
///
/// Valida i parametri, sceglie la granularità e compone le tre aggregazioni
/// (totali, per sport, serie per periodo). Non conosce HTTP.
use axum::http::StatusCode;
use chrono::NaiveDate;
use sqlx::PgPool;

use crate::dto::stats::{StatsQuery, StatsSummaryDto};
use crate::errors::AppError;
use crate::middleware::AuthUser;
use crate::parsing::Sport;
use crate::repositories::stats_repository::{self, StatsFilter};
use crate::repositories::user_repository;
use crate::services::access_service;

/// Ampiezza massima dell'intervallo, in giorni (estremi inclusi).
pub const MAX_RANGE_DAYS: i64 = 3660;

/// Ampiezza massima (in giorni) per cui la granularità automatica è `day`.
const AUTO_DAY_MAX_DAYS: i64 = 31;
/// Ampiezza massima (in giorni) per cui la granularità automatica è `week`.
const AUTO_WEEK_MAX_DAYS: i64 = 180;

/// Ampiezza del periodo di aggregazione della serie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Granularity {
    Day,
    Week,
    Month,
}

impl Granularity {
    /// Valore testuale, anche argomento di `date_trunc` a DB.
    pub fn as_str(&self) -> &'static str {
        match self {
            Granularity::Day => "day",
            Granularity::Week => "week",
            Granularity::Month => "month",
        }
    }

    /// `None` se il valore non è `day`, `week` o `month` (case-sensitive).
    pub fn from_str_opt(s: &str) -> Option<Granularity> {
        match s {
            "day" => Some(Granularity::Day),
            "week" => Some(Granularity::Week),
            "month" => Some(Granularity::Month),
            _ => None,
        }
    }

    /// Granularità di default per un intervallo di `days` giorni.
    pub fn auto(days: i64) -> Granularity {
        if days <= AUTO_DAY_MAX_DAYS {
            Granularity::Day
        } else if days <= AUTO_WEEK_MAX_DAYS {
            Granularity::Week
        } else {
            Granularity::Month
        }
    }
}

/// Richiesta di statistiche già validata.
pub struct SummaryRequest {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub sport: Option<String>,
    pub granularity: Granularity,
    /// Atleta di cui leggere i dati (ancora da autorizzare)
    pub athlete_id: Option<i64>,
}

fn validation_error(message: &str) -> AppError {
    AppError::coded(StatusCode::BAD_REQUEST, "VALIDATION_FAILED", message)
}

/// Valida la query string e risolve la granularità.
///
/// Errori `400 VALIDATION_FAILED`: `from`/`to` mancanti, `from > to`,
/// intervallo oltre `MAX_RANGE_DAYS`, sport o granularità non validi.
fn validate(query: StatsQuery) -> Result<SummaryRequest, AppError> {
    let from = query
        .from
        .ok_or_else(|| validation_error("from è obbligatorio"))?;
    let to = query
        .to
        .ok_or_else(|| validation_error("to è obbligatorio"))?;
    if from > to {
        return Err(validation_error("from non può essere successivo a to"));
    }
    let days = (to - from).num_days() + 1;
    if days > MAX_RANGE_DAYS {
        return Err(validation_error(
            "l'intervallo non può superare 3660 giorni",
        ));
    }
    if let Some(sport) = query.sport.as_deref()
        && Sport::from_str_opt(sport).is_none()
    {
        return Err(validation_error("sport non valido"));
    }
    let granularity = match query.granularity.as_deref() {
        Some(g) => Granularity::from_str_opt(g)
            .ok_or_else(|| validation_error("granularity deve essere day, week o month"))?,
        None => Granularity::auto(days),
    };

    Ok(SummaryRequest {
        from,
        to,
        sport: query.sport,
        granularity,
        athlete_id: query.athlete_id,
    })
}

/// Statistiche dell'utente o di un atleta collegato (`athlete_id`).
///
/// Un input errato è 400 per chiunque, prima del controllo d'accesso.
pub async fn get_summary(
    pool: &PgPool,
    auth: &AuthUser,
    query: StatsQuery,
) -> Result<StatsSummaryDto, AppError> {
    let request = validate(query)?;

    let target = access_service::resolve_target(pool, auth, request.athlete_id).await?;
    // Giorni e periodi sono calcolati nel fuso dell'atleta di cui si leggono i dati
    let timezone = user_repository::timezone_of(pool, target).await?;

    summary(pool, target, &timezone, &request).await
}

/// Statistiche dell'atleta `user_id`, con giorni e periodi nel suo fuso `timezone`.
async fn summary(
    pool: &PgPool,
    user_id: i64,
    timezone: &str,
    request: &SummaryRequest,
) -> Result<StatsSummaryDto, AppError> {
    let filter = StatsFilter {
        from: request.from,
        to: request.to,
        sport: request.sport.as_deref(),
        timezone,
    };
    let totals = stats_repository::totals(pool, user_id, &filter).await?;
    let by_sport = stats_repository::by_sport(pool, user_id, &filter).await?;
    let series =
        stats_repository::series(pool, user_id, &filter, request.granularity.as_str()).await?;

    Ok(StatsSummaryDto {
        from: request.from,
        to: request.to,
        granularity: request.granularity.as_str(),
        timezone: timezone.to_string(),
        totals,
        by_sport,
        series,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_granularity_thresholds() {
        assert_eq!(Granularity::auto(1), Granularity::Day);
        assert_eq!(Granularity::auto(31), Granularity::Day);
        assert_eq!(Granularity::auto(32), Granularity::Week);
        assert_eq!(Granularity::auto(180), Granularity::Week);
        assert_eq!(Granularity::auto(181), Granularity::Month);
    }

    #[test]
    fn granularity_round_trips_through_as_str() {
        for g in [Granularity::Day, Granularity::Week, Granularity::Month] {
            assert_eq!(Granularity::from_str_opt(g.as_str()), Some(g));
        }
        assert_eq!(Granularity::from_str_opt("year"), None);
        assert_eq!(Granularity::from_str_opt("Day"), None);
    }
}
