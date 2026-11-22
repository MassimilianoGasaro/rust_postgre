/// dto/activity.rs — DTO delle attività

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::models::activity::Activity;

/// Query string di `GET /activities`.
///
/// Un valore malformato (data non valida, `page` non numerico...) è rifiutato
/// da `AppQuery` con `400 VALIDATION_FAILED`.
#[derive(Debug, Deserialize)]
pub struct ListActivitiesQuery {
    /// Prima data inclusa (`YYYY-MM-DD`, nel fuso dell'atleta)
    pub from: Option<NaiveDate>,
    /// Ultima data inclusa (`YYYY-MM-DD`, nel fuso dell'atleta)
    pub to: Option<NaiveDate>,
    pub sport: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    /// Atleta di cui leggere le attività (solo coach con link attivo)
    pub athlete_id: Option<i64>,
}

/// Attività nelle risposte API (senza percorso né hash del file originale).
#[derive(Debug, Serialize)]
pub struct ActivityDto {
    pub id: i64,
    pub source: String,
    pub sport: String,
    pub sport_raw: String,
    pub name: Option<String>,
    /// Inizio in UTC (RFC 3339)
    pub start_time: DateTime<Utc>,
    pub utc_offset_seconds: Option<i32>,
    pub elapsed_s: i32,
    pub moving_s: Option<i32>,
    pub distance_m: Option<f64>,
    pub elevation_gain_m: Option<f64>,
    pub avg_hr: Option<i32>,
    pub max_hr: Option<i32>,
    pub avg_speed_mps: Option<f64>,
    pub avg_power_w: Option<i32>,
    pub avg_cadence: Option<i32>,
    pub calories: Option<i32>,
    pub file_format: String,
}

/// Pagina di attività con il totale dei risultati (a parità di filtri).
#[derive(Debug, Serialize)]
pub struct ActivityPageDto {
    pub items: Vec<ActivityDto>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
}

impl From<Activity> for ActivityDto {
    fn from(a: Activity) -> Self {
        ActivityDto {
            id: a.id,
            source: a.source,
            sport: a.sport,
            sport_raw: a.sport_raw,
            name: a.name,
            start_time: a.start_time,
            utc_offset_seconds: a.utc_offset_seconds,
            elapsed_s: a.elapsed_s,
            moving_s: a.moving_s,
            distance_m: a.distance_m,
            elevation_gain_m: a.elevation_gain_m,
            avg_hr: a.avg_hr,
            max_hr: a.max_hr,
            avg_speed_mps: a.avg_speed_mps,
            avg_power_w: a.avg_power_w,
            avg_cadence: a.avg_cadence,
            calories: a.calories,
            file_format: a.file_format,
        }
    }
}
