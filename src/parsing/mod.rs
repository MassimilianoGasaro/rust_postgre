//! parsing — Lettura dei file di attività (FIT, GPX, TCX)
//!
//! Modulo puro: nessun accesso a DB o HTTP. Ogni formato produce una
//! `ParsedActivity` con le metriche aggregate dell'attività.

pub mod archive;
pub mod fit;
pub mod gpx;
pub mod metrics;
pub mod tcx;

use std::fmt;

use chrono::{DateTime, Utc};

/// Sport normalizzato dell'attività.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sport {
    Running,
    Cycling,
    Swimming,
    Walking,
    Hiking,
    Strength,
    Other,
}

impl Sport {
    /// Tutti gli sport normalizzati, nell'ordine di dichiarazione.
    pub const ALL: [Sport; 7] = [
        Sport::Running,
        Sport::Cycling,
        Sport::Swimming,
        Sport::Walking,
        Sport::Hiking,
        Sport::Strength,
        Sport::Other,
    ];

    /// Inverso di `as_str`: `None` se il valore non è uno sport normalizzato
    /// (il confronto è case-sensitive, come i valori salvati a DB).
    pub fn from_str_opt(s: &str) -> Option<Sport> {
        Sport::ALL.into_iter().find(|sport| sport.as_str() == s)
    }

    /// Valore testuale salvato a DB e restituito dalle API.
    pub fn as_str(&self) -> &'static str {
        match self {
            Sport::Running => "running",
            Sport::Cycling => "cycling",
            Sport::Swimming => "swimming",
            Sport::Walking => "walking",
            Sport::Hiking => "hiking",
            Sport::Strength => "strength",
            Sport::Other => "other",
        }
    }
}

/// Formato di file di attività supportato.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileFormat {
    Fit,
    Gpx,
    Tcx,
}

impl FileFormat {
    /// Deduce il formato dall'estensione del nome file (case-insensitive).
    pub fn from_filename(name: &str) -> Option<FileFormat> {
        let (_, ext) = name.rsplit_once('.')?;
        match ext.to_ascii_lowercase().as_str() {
            "fit" => Some(FileFormat::Fit),
            "gpx" => Some(FileFormat::Gpx),
            "tcx" => Some(FileFormat::Tcx),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            FileFormat::Fit => "fit",
            FileFormat::Gpx => "gpx",
            FileFormat::Tcx => "tcx",
        }
    }
}

/// Attività letta da un file, con le metriche aggregate.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedActivity {
    pub sport: Sport,
    /// Valore dello sport così come scritto nel file
    pub sport_raw: String,
    pub name: Option<String>,
    pub start_time: DateTime<Utc>,
    pub utc_offset_seconds: Option<i32>,
    /// Durata complessiva (dall'inizio all'ultimo punto)
    pub elapsed_s: i32,
    /// Tempo in movimento
    pub moving_s: Option<i32>,
    pub distance_m: Option<f64>,
    pub elevation_gain_m: Option<f64>,
    pub avg_hr: Option<i32>,
    pub max_hr: Option<i32>,
    pub avg_speed_mps: Option<f64>,
    pub avg_power_w: Option<i32>,
    pub avg_cadence: Option<i32>,
    pub calories: Option<i32>,
}

/// Errore di parsing di un file di attività.
#[derive(Debug)]
pub enum ParseError {
    /// File non leggibile o non conforme al formato
    Malformed(String),
    /// File valido ma senza dati utilizzabili
    Empty,
}

impl ParseError {
    /// Codice applicativo esposto nella risposta di errore.
    pub fn code(&self) -> &'static str {
        match self {
            ParseError::Malformed(_) => "PARSE_ERROR",
            ParseError::Empty => "EMPTY_ACTIVITY",
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Malformed(msg) => write!(f, "file malformed: {msg}"),
            ParseError::Empty => write!(f, "file contains no usable activity data"),
        }
    }
}

impl std::error::Error for ParseError {}

impl From<quick_xml::Error> for ParseError {
    fn from(e: quick_xml::Error) -> Self {
        ParseError::Malformed(e.to_string())
    }
}

/// Legge un file di attività nel formato indicato.
pub fn parse(format: FileFormat, bytes: &[u8]) -> Result<ParsedActivity, ParseError> {
    match format {
        FileFormat::Gpx => gpx::parse(bytes),
        FileFormat::Tcx => tcx::parse(bytes),
        FileFormat::Fit => fit::parse(bytes),
    }
}

/// Profondità massima di annidamento XML accettata (difesa da input ostili).
pub(crate) const MAX_XML_DEPTH: usize = 64;

/// Lunghezza massima (in caratteri) dei testi liberi letti dai file XML
/// (nome dell'attività, sport così come scritto): l'input non è fidato.
pub(crate) const MAX_TEXT_CHARS: usize = 200;

/// Testo limitato a `MAX_TEXT_CHARS` caratteri (non byte).
pub(crate) fn truncate_text(s: &str) -> String {
    s.chars().take(MAX_TEXT_CHARS).collect()
}

/// Interpreta un timestamp RFC 3339 e lo converte in UTC.
pub(crate) fn parse_time(s: &str) -> Result<DateTime<Utc>, ParseError> {
    DateTime::parse_from_rfc3339(s.trim())
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| {
            // Il testo riportato nell'errore è troncato: l'input non è fidato
            let shown: String = s.trim().chars().take(40).collect();
            ParseError::Malformed(format!("invalid timestamp '{shown}': {e}"))
        })
}

/// Numero di secondi (anche frazionari) → i32, saturando invece di troncare.
pub(crate) fn seconds_to_i32(seconds: f64) -> i32 {
    i32::try_from(seconds.round() as i64).unwrap_or(i32::MAX)
}

/// Verifica la profondità di annidamento prima di aprire un nuovo elemento.
pub(crate) fn check_depth(depth: usize) -> Result<(), ParseError> {
    if depth >= MAX_XML_DEPTH {
        return Err(ParseError::Malformed(format!(
            "xml nesting deeper than {MAX_XML_DEPTH} levels"
        )));
    }
    Ok(())
}

/// Parsing di un numero finito (NaN e infiniti sono trattati come assenti).
pub(crate) fn finite_number(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Local name (senza prefisso di namespace) di un elemento XML.
pub(crate) fn local_name(raw: quick_xml::name::LocalName<'_>) -> String {
    String::from_utf8_lossy(raw.as_ref()).into_owned()
}

/// Media pesata; se i pesi sono tutti nulli ripiega sulla media semplice.
pub(crate) fn weighted_mean(values: &[(f64, f64)]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let total_weight: f64 = values.iter().map(|(_, w)| w).sum();
    if total_weight > 0.0 {
        Some(values.iter().map(|(v, w)| v * w).sum::<f64>() / total_weight)
    } else {
        Some(values.iter().map(|(v, _)| v).sum::<f64>() / values.len() as f64)
    }
}

/// Media aritmetica arrotondata all'intero più vicino (None se vuota).
pub(crate) fn rounded_mean(values: &[f64]) -> Option<i32> {
    if values.is_empty() {
        return None;
    }
    Some((values.iter().sum::<f64>() / values.len() as f64).round() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_filename_is_case_insensitive() {
        assert_eq!(FileFormat::from_filename("a.GPX"), Some(FileFormat::Gpx));
        assert_eq!(
            FileFormat::from_filename("dir.v2/a.Tcx"),
            Some(FileFormat::Tcx)
        );
        assert_eq!(FileFormat::from_filename("a.fit"), Some(FileFormat::Fit));
        assert_eq!(FileFormat::from_filename("a.zip"), None);
        assert_eq!(FileFormat::from_filename("noext"), None);
    }

    #[test]
    fn as_str_values() {
        assert_eq!(Sport::Cycling.as_str(), "cycling");
        assert_eq!(FileFormat::Tcx.as_str(), "tcx");
        assert_eq!(ParseError::Empty.code(), "EMPTY_ACTIVITY");
        assert_eq!(ParseError::Malformed("x".into()).code(), "PARSE_ERROR");
    }

    #[test]
    fn sport_all_round_trips_through_as_str() {
        for sport in Sport::ALL {
            assert_eq!(Sport::from_str_opt(sport.as_str()), Some(sport));
        }
        assert_eq!(Sport::from_str_opt("Running"), None);
        assert_eq!(Sport::from_str_opt("yoga"), None);
        assert_eq!(Sport::from_str_opt(""), None);
    }

    #[test]
    fn parse_garbage_is_malformed() {
        for format in [FileFormat::Gpx, FileFormat::Tcx] {
            let err = parse(format, b"not xml").unwrap_err();
            assert_eq!(err.code(), "PARSE_ERROR");
        }
    }

    #[test]
    fn parse_time_error_truncates_echoed_text() {
        let long = "x".repeat(500);
        let ParseError::Malformed(msg) = parse_time(&long).unwrap_err() else {
            panic!("expected Malformed");
        };
        assert!(msg.len() < 200, "len = {}", msg.len());
    }

    #[test]
    fn parse_wrong_root_is_malformed() {
        let gpx = include_bytes!("../../tests/fixtures/run.gpx");
        assert_eq!(
            parse(FileFormat::Tcx, gpx).unwrap_err().code(),
            "PARSE_ERROR"
        );
    }
}
