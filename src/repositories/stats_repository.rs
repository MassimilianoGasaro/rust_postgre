/// repositories/stats_repository.rs — Query SQL di aggregazione delle statistiche
///
/// Tutte le query filtrano per utente, intervallo di date (inclusivo, nel fuso
/// `timezone`) e sport opzionale; `moving_s` ricade su `elapsed_s` se assente
/// e le metriche nulle valgono 0.

use chrono::NaiveDate;
use sqlx::PgExecutor;

use crate::dto::stats::{PeriodStatsDto, SportStatsDto, TotalsDto};
use crate::errors::AppError;

/// Criteri comuni alle tre aggregazioni.
pub struct StatsFilter<'a> {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub sport: Option<&'a str>,
    /// Fuso IANA dell'atleta, con cui `start_time` viene convertito in ora locale
    pub timezone: &'a str,
}

/// Totali dell'intero intervallo (una riga, a zero se non ci sono attività).
pub async fn totals<'e>(
    executor: impl PgExecutor<'e>,
    user_id: i64,
    filter: &StatsFilter<'_>,
) -> Result<TotalsDto, AppError> {
    let row = sqlx::query_as!(
        TotalsDto,
        r#"
        SELECT COUNT(*)                                              AS "count!",
               COALESCE(SUM(distance_m), 0)::float8                  AS "distance_m!",
               COALESCE(SUM(COALESCE(moving_s, elapsed_s)), 0)::int8 AS "moving_s!",
               COALESCE(SUM(elapsed_s), 0)::int8                     AS "elapsed_s!",
               COALESCE(SUM(elevation_gain_m), 0)::float8            AS "elevation_gain_m!"
        FROM activities
        WHERE user_id = $1
          AND (start_time AT TIME ZONE $4)::date >= $2
          AND (start_time AT TIME ZONE $4)::date <= $3
          AND ($5::text IS NULL OR sport = $5)
        "#,
        user_id,
        filter.from,
        filter.to,
        filter.timezone,
        filter.sport
    )
    .fetch_one(executor)
    .await?;

    Ok(row)
}

/// Totali per sport, dal più praticato (`moving_s` decrescente, poi nome sport).
pub async fn by_sport<'e>(
    executor: impl PgExecutor<'e>,
    user_id: i64,
    filter: &StatsFilter<'_>,
) -> Result<Vec<SportStatsDto>, AppError> {
    let rows = sqlx::query_as!(
        SportStatsDto,
        r#"
        SELECT sport                                                 AS "sport!",
               COUNT(*)                                              AS "count!",
               COALESCE(SUM(distance_m), 0)::float8                  AS "distance_m!",
               COALESCE(SUM(COALESCE(moving_s, elapsed_s)), 0)::int8 AS "moving_s!"
        FROM activities
        WHERE user_id = $1
          AND (start_time AT TIME ZONE $4)::date >= $2
          AND (start_time AT TIME ZONE $4)::date <= $3
          AND ($5::text IS NULL OR sport = $5)
        GROUP BY sport
        ORDER BY 4 DESC, sport
        "#,
        user_id,
        filter.from,
        filter.to,
        filter.timezone,
        filter.sport
    )
    .fetch_all(executor)
    .await?;

    Ok(rows)
}

/// Serie per periodo (`granularity` = `day`, `week` o `month`, già validata).
///
/// I periodi sono generati in SQL da `date_trunc(g, from)` a `to` e uniti in
/// LEFT JOIN alle attività aggregate: i periodi senza attività valgono 0.
/// Le settimane sono ISO (inizio lunedì, comportamento di `date_trunc('week')`).
pub async fn series<'e>(
    executor: impl PgExecutor<'e>,
    user_id: i64,
    filter: &StatsFilter<'_>,
    granularity: &str,
) -> Result<Vec<PeriodStatsDto>, AppError> {
    let rows = sqlx::query_as!(
        PeriodStatsDto,
        r#"
        WITH periods AS (
            SELECT generate_series(
                date_trunc($5::text, $2::date::timestamp),
                $3::date::timestamp,
                ('1 ' || $5::text)::interval
            ) AS period
        ),
        agg AS (
            SELECT date_trunc($5::text, start_time AT TIME ZONE $4) AS period,
                   COUNT(*)                                         AS count,
                   SUM(distance_m)                                  AS distance_m,
                   SUM(COALESCE(moving_s, elapsed_s))               AS moving_s,
                   SUM(elevation_gain_m)                            AS elevation_gain_m
            FROM activities
            WHERE user_id = $1
              AND (start_time AT TIME ZONE $4)::date >= $2
              AND (start_time AT TIME ZONE $4)::date <= $3
              AND ($6::text IS NULL OR sport = $6)
            GROUP BY 1
        )
        SELECT periods.period::date                      AS "period_start!",
               COALESCE(agg.count, 0)                    AS "count!",
               COALESCE(agg.distance_m, 0)::float8       AS "distance_m!",
               COALESCE(agg.moving_s, 0)::int8           AS "moving_s!",
               COALESCE(agg.elevation_gain_m, 0)::float8 AS "elevation_gain_m!"
        FROM periods
        LEFT JOIN agg ON agg.period = periods.period
        ORDER BY periods.period
        "#,
        user_id,
        filter.from,
        filter.to,
        filter.timezone,
        granularity,
        filter.sport
    )
    .fetch_all(executor)
    .await?;

    Ok(rows)
}
