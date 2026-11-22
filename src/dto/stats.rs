/// dto/stats.rs — DTO delle statistiche
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Query string di `GET /stats/summary`.
///
/// `from` e `to` sono `Option` solo per poter rispondere con un errore
/// codificato (`400 VALIDATION_FAILED`) quando mancano: il service li esige.
#[derive(Debug, Deserialize)]
pub struct StatsQuery {
    /// Prima data inclusa (`YYYY-MM-DD`, nel fuso dell'atleta)
    pub from: Option<NaiveDate>,
    /// Ultima data inclusa (`YYYY-MM-DD`, nel fuso dell'atleta)
    pub to: Option<NaiveDate>,
    pub sport: Option<String>,
    /// `day`, `week` o `month`; se assente dipende dall'ampiezza dell'intervallo
    pub granularity: Option<String>,
    /// Atleta di cui leggere le statistiche (solo coach con link attivo)
    pub athlete_id: Option<i64>,
}

/// Totali dell'intero intervallo.
#[derive(Debug, Serialize)]
pub struct TotalsDto {
    pub count: i64,
    pub distance_m: f64,
    pub moving_s: i64,
    pub elapsed_s: i64,
    pub elevation_gain_m: f64,
}

/// Totali di un singolo sport.
#[derive(Debug, Serialize)]
pub struct SportStatsDto {
    pub sport: String,
    pub count: i64,
    pub distance_m: f64,
    pub moving_s: i64,
}

/// Totali di un periodo (giorno, settimana o mese) della serie.
#[derive(Debug, Serialize)]
pub struct PeriodStatsDto {
    /// Primo giorno del periodo (`YYYY-MM-DD`)
    pub period_start: NaiveDate,
    pub count: i64,
    pub distance_m: f64,
    pub moving_s: i64,
    pub elevation_gain_m: f64,
}

/// Risposta di `GET /stats/summary`.
#[derive(Debug, Serialize)]
pub struct StatsSummaryDto {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub granularity: &'static str,
    /// Fuso dell'atleta in cui sono calcolati giorni e periodi
    pub timezone: String,
    pub totals: TotalsDto,
    pub by_sport: Vec<SportStatsDto>,
    pub series: Vec<PeriodStatsDto>,
}
