//! parsing — Lettura dei file di attività (GPX, TCX; FIT nel Task 5)
//!
//! Modulo puro: nessun accesso a DB o HTTP. Ogni formato produce una
//! `ParsedActivity` con le metriche aggregate dell'attività.

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
        FileFormat::Fit => Err(ParseError::Malformed("fit not yet supported".into())),
    }
}

/// Interpreta un timestamp RFC 3339 e lo converte in UTC.
pub(crate) fn parse_time(s: &str) -> Result<DateTime<Utc>, ParseError> {
    DateTime::parse_from_rfc3339(s.trim())
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| ParseError::Malformed(format!("invalid timestamp '{}': {e}", s.trim())))
}

/// Local name (senza prefisso di namespace) di un elemento XML.
pub(crate) fn local_name(raw: quick_xml::name::LocalName<'_>) -> String {
    String::from_utf8_lossy(raw.as_ref()).into_owned()
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
        assert_eq!(FileFormat::from_filename("dir.v2/a.Tcx"), Some(FileFormat::Tcx));
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
    fn parse_garbage_is_malformed() {
        for format in [FileFormat::Gpx, FileFormat::Tcx] {
            let err = parse(format, b"not xml").unwrap_err();
            assert_eq!(err.code(), "PARSE_ERROR");
        }
    }

    #[test]
    fn parse_wrong_root_is_malformed() {
        let gpx = include_bytes!("../../tests/fixtures/run.gpx");
        assert_eq!(parse(FileFormat::Tcx, gpx).unwrap_err().code(), "PARSE_ERROR");
    }
}
