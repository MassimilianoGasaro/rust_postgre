/// models/activity.rs — Il Model dell'attività sportiva
///
/// Rispecchia la tabella `activities`: una riga per ogni file importato
/// (FIT, GPX, TCX) con le metriche aggregate calcolate dal parser.
use chrono::{DateTime, Utc};

/// Riga della tabella `activities`.
#[derive(Debug, Clone)]
pub struct Activity {
    pub id: i64,
    pub user_id: i64,
    /// Provenienza: `file_upload` (unico valore usato finora), `garmin`, `coros`, `strava`
    pub source: String,
    /// Sport normalizzato (vedi `parsing::Sport::as_str`)
    pub sport: String,
    /// Sport così come scritto nel file
    pub sport_raw: String,
    pub name: Option<String>,
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
    /// `fit`, `gpx` o `tcx`
    pub file_format: String,
    /// Percorso del file originale, relativo a `UPLOAD_DIR`
    pub file_path: String,
    pub file_sha256: String,
    pub created_at: DateTime<Utc>,
}
