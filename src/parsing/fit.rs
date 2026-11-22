//! fit — Parsing FIT
//!
//! Decodifica con `fitparser` in streaming: dei messaggi decodificati si
//! conservano solo `session` e `activity`, così un file molto grande non
//! viene materializzato per intero in memoria. Le metriche sono aggregate
//! sulle sessioni (un file può contenerne più d'una, ad es. un triathlon).

use std::panic::{AssertUnwindSafe, catch_unwind};

use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use fitparser::de::{FitObject, FitStreamProcessor};
use fitparser::profile::MesgNum;
use fitparser::{FitDataRecord, Value};

use super::{ParseError, ParsedActivity, Sport, seconds_to_i32, weighted_mean};

/// Numero massimo di sessioni accettate (difesa da input ostili).
const MAX_SESSIONS: usize = 1000;
/// Scarto massimo plausibile tra ora locale e UTC (ore).
const MAX_UTC_OFFSET_HOURS: i64 = 18;
/// Lunghezza massima dei nomi di sport riportati in `sport_raw`.
const MAX_SPORT_CHARS: usize = 40;

/// Dati di una sessione (`session`) rilevanti per le metriche.
#[derive(Default)]
struct Session {
    start: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
    sport: Option<String>,
    sub_sport: Option<String>,
    elapsed_s: Option<f64>,
    timer_s: Option<f64>,
    distance_m: Option<f64>,
    ascent_m: Option<f64>,
    calories: Option<f64>,
    avg_hr: Option<f64>,
    max_hr: Option<f64>,
    avg_speed: Option<f64>,
    avg_power: Option<f64>,
    avg_cadence: Option<f64>,
}

/// Valore numerico finito di un campo (NaN, infiniti e non numerici sono assenti).
fn number(value: &Value) -> Option<f64> {
    let n = match value {
        Value::Byte(v) | Value::Enum(v) | Value::UInt8(v) | Value::UInt8z(v) => f64::from(*v),
        Value::SInt8(v) => f64::from(*v),
        Value::SInt16(v) => f64::from(*v),
        Value::UInt16(v) | Value::UInt16z(v) => f64::from(*v),
        Value::SInt32(v) => f64::from(*v),
        Value::UInt32(v) | Value::UInt32z(v) => f64::from(*v),
        Value::Float32(v) => f64::from(*v),
        Value::Float64(v) => *v,
        // I valori a 64 bit perdono precisione oltre 2^53: irrilevante per queste metriche
        Value::SInt64(v) => *v as f64,
        Value::UInt64(v) | Value::UInt64z(v) => *v as f64,
        _ => return None,
    };
    n.is_finite().then_some(n)
}

/// Valore numerico non negativo (durate, distanze, conteggi).
fn non_negative(value: &Value) -> Option<f64> {
    number(value).filter(|n| *n >= 0.0)
}

fn timestamp(value: &Value) -> Option<DateTime<Utc>> {
    match value {
        Value::Timestamp(t) => Some(t.with_timezone(&Utc)),
        _ => None,
    }
}

/// Testo di un campo enumerato, troncato perché l'input non è fidato.
fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() => Some(s.chars().take(MAX_SPORT_CHARS).collect()),
        _ => None,
    }
}

impl Session {
    fn from_record(record: &FitDataRecord) -> Session {
        let mut s = Session::default();
        for field in record.fields() {
            let v = field.value();
            match field.name() {
                "start_time" => s.start = timestamp(v),
                "timestamp" => s.end = timestamp(v),
                "sport" => s.sport = text(v),
                "sub_sport" => s.sub_sport = text(v),
                "total_elapsed_time" => s.elapsed_s = non_negative(v),
                "total_timer_time" => s.timer_s = non_negative(v),
                "total_distance" => s.distance_m = non_negative(v),
                "total_ascent" => s.ascent_m = non_negative(v),
                "total_calories" => s.calories = non_negative(v),
                "avg_heart_rate" => s.avg_hr = non_negative(v),
                "max_heart_rate" => s.max_hr = non_negative(v),
                // `enhanced_avg_speed` ha precedenza su `avg_speed`, in qualsiasi ordine
                "enhanced_avg_speed" => s.avg_speed = non_negative(v).or(s.avg_speed),
                "avg_speed" => s.avg_speed = s.avg_speed.or_else(|| non_negative(v)),
                "avg_power" => s.avg_power = non_negative(v),
                "avg_cadence" => s.avg_cadence = non_negative(v),
                // Corsa: il FIT conta le falcate (un solo piede), 2 falcate = 2 passi
                "avg_running_cadence" if s.avg_cadence.is_none() => {
                    s.avg_cadence = non_negative(v).map(|c| c * 2.0);
                }
                _ => {}
            }
        }
        s
    }

    /// Durata complessiva: campo dedicato, altrimenti timer, altrimenti fine - inizio.
    fn elapsed(&self) -> f64 {
        self.elapsed_s
            .or(self.timer_s)
            .or_else(|| match (self.start, self.end) {
                (Some(start), Some(end)) => Some((end - start).num_seconds().max(0) as f64),
                _ => None,
            })
            .unwrap_or(0.0)
    }

    /// Peso per le medie: il tempo in movimento.
    fn weight(&self) -> f64 {
        self.timer_s.unwrap_or(0.0)
    }
}

/// Sport normalizzato da `sport` e `sub_sport` del FIT.
fn normalize_sport(sport: Option<&str>, sub_sport: Option<&str>) -> Sport {
    match (sport, sub_sport) {
        (Some("training"), Some("strength_training")) => Sport::Strength,
        (Some("running"), _) => Sport::Running,
        (Some("cycling"), _) => Sport::Cycling,
        (Some("swimming"), _) => Sport::Swimming,
        (Some("walking"), _) => Sport::Walking,
        (Some("hiking"), _) => Sport::Hiking,
        _ => Sport::Other,
    }
}

/// Decodifica il file conservando solo le sessioni e lo scarto ora locale/UTC.
fn collect(bytes: &[u8]) -> Result<(Vec<Session>, Option<i32>), ParseError> {
    let malformed = |e: fitparser::Error| ParseError::Malformed(e.to_string());
    let mut processor = FitStreamProcessor::new();
    let mut sessions = Vec::new();
    let mut utc_offset = None;
    let mut rest = bytes;
    // Il decoder va in panic se la dimensione dichiarata dell'header supera i dati:
    // la si verifica prima di ogni header (all'inizio e dopo ogni CRC finale)
    let mut at_header = true;

    while !rest.is_empty() {
        if at_header {
            if !matches!(rest.first(), Some(12 | 14)) {
                return Err(ParseError::Malformed("invalid fit header size".into()));
            }
            at_header = false;
        }
        let (next, object) = processor.deserialize_next(rest).map_err(malformed)?;
        // Il decoder deve sempre avanzare: altrimenti il ciclo non terminerebbe
        if next.len() >= rest.len() {
            return Err(ParseError::Malformed("decoder made no progress".into()));
        }
        rest = next;
        match object {
            FitObject::Crc(..) => {
                processor.reset();
                at_header = true;
            }
            FitObject::DataMessage(message) => {
                // Va decodificato ogni messaggio: gli orari compressi dipendono dai precedenti
                let record = processor.decode_message(message).map_err(malformed)?;
                match record.kind() {
                    MesgNum::Session => {
                        if sessions.len() >= MAX_SESSIONS {
                            return Err(ParseError::Malformed(format!(
                                "more than {MAX_SESSIONS} sessions"
                            )));
                        }
                        sessions.push(Session::from_record(&record));
                    }
                    MesgNum::Activity if utc_offset.is_none() => {
                        utc_offset = offset_from_activity(&record);
                    }
                    _ => {}
                }
            }
            FitObject::Header(..) | FitObject::DefinitionMessage(..) => {}
        }
    }
    Ok((sessions, utc_offset))
}

/// Epoca FIT (1989-12-31T00:00:00Z) in secondi Unix.
const FIT_EPOCH_UNIX: i64 = 631_065_600;

/// Epoca FIT interpretata nel fuso locale, come fa `fitparser` per `local_timestamp`.
fn local_epoch() -> Option<i64> {
    let reference = NaiveDate::from_ymd_opt(1989, 12, 31)?.and_hms_opt(0, 0, 0)?;
    Local
        .from_local_datetime(&reference)
        .single()
        .map(|t| t.timestamp())
}

/// `local_timestamp - timestamp` del messaggio `activity`, se plausibile.
fn offset_from_activity(record: &FitDataRecord) -> Option<i32> {
    let mut local = None;
    let mut utc = None;
    for field in record.fields() {
        match field.name() {
            "local_timestamp" => local = timestamp(field.value()),
            "timestamp" => utc = timestamp(field.value()),
            _ => {}
        }
    }
    // `fitparser` legge `local_timestamp` come ora locale del fuso della macchina:
    // si risale ai secondi grezzi dall'epoca FIT sottraendo lo stesso riferimento
    let local_secs = local?.timestamp().checked_sub(local_epoch()?)?;
    let utc_secs = utc?.timestamp().checked_sub(FIT_EPOCH_UNIX)?;
    let delta = local_secs.saturating_sub(utc_secs);
    // Alcuni dispositivi scrivono un `local_timestamp` non valido (es. epoca FIT): si scarta
    (delta.abs() <= MAX_UTC_OFFSET_HOURS * 3600).then_some(delta as i32)
}

/// Legge un file FIT e ne calcola le metriche.
pub fn parse(bytes: &[u8]) -> Result<ParsedActivity, ParseError> {
    // Difesa in profondità: un panic del decoder su input ostile diventa un errore
    let (sessions, utc_offset_seconds) = catch_unwind(AssertUnwindSafe(|| collect(bytes)))
        .map_err(|_| ParseError::Malformed("fit decoder failed".into()))??;
    build(&sessions, utc_offset_seconds)
}

/// Aggrega le sessioni in un'unica attività.
fn build(
    sessions: &[Session],
    utc_offset_seconds: Option<i32>,
) -> Result<ParsedActivity, ParseError> {
    // Più sessioni: lo sport è quello della sessione più lunga (a parità, la prima)
    let Some(main) = sessions.iter().reduce(|best, s| {
        if s.elapsed() > best.elapsed() {
            s
        } else {
            best
        }
    }) else {
        return Err(ParseError::Empty);
    };
    let Some(start_time) = sessions.iter().filter_map(|s| s.start).min() else {
        return Err(ParseError::Empty);
    };

    let sum = |pick: fn(&Session) -> Option<f64>| -> Option<f64> {
        let values: Vec<f64> = sessions.iter().filter_map(pick).collect();
        (!values.is_empty()).then(|| values.iter().sum())
    };
    let weighted = |pick: fn(&Session) -> Option<f64>| -> Option<f64> {
        let values: Vec<(f64, f64)> = sessions
            .iter()
            .filter_map(|s| pick(s).map(|v| (v, s.weight())))
            .collect();
        weighted_mean(&values)
    };

    let elapsed_s = seconds_to_i32(sessions.iter().map(Session::elapsed).sum());
    let moving_s = sum(|s| s.timer_s).map(seconds_to_i32);
    let distance_m = sum(|s| s.distance_m);
    let avg_speed_mps = weighted(|s| s.avg_speed).or_else(|| match (distance_m, moving_s) {
        (Some(d), Some(m)) if m > 0 => Some(d / f64::from(m)),
        _ => None,
    });
    // `as` satura e NaN diventa 0; i valori sono comunque finiti
    let round = |v: f64| v.round() as i32;

    let sport_name = main.sport.as_deref();
    let sub_sport_name = main.sub_sport.as_deref();
    let sport_raw = match (sport_name, sub_sport_name) {
        (Some(sport), Some(sub)) => format!("{sport}/{sub}"),
        (Some(sport), None) => sport.to_string(),
        (None, _) => "unknown".to_string(),
    };

    Ok(ParsedActivity {
        sport: normalize_sport(sport_name, sub_sport_name),
        sport_raw,
        name: None,
        start_time,
        utc_offset_seconds,
        elapsed_s,
        moving_s,
        distance_m,
        elevation_gain_m: sum(|s| s.ascent_m),
        avg_hr: weighted(|s| s.avg_hr).map(round),
        max_hr: sessions
            .iter()
            .filter_map(|s| s.max_hr)
            .reduce(f64::max)
            .map(round),
        avg_speed_mps,
        avg_power_w: weighted(|s| s.avg_power).map(round),
        avg_cadence: weighted(|s| s.avg_cadence).map(round),
        calories: sum(|s| s.calories).map(round),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Secondi tra l'epoca Unix e l'epoca FIT (1989-12-31T00:00:00Z).
    const FIT_EPOCH: i64 = FIT_EPOCH_UNIX;

    fn crc16(data: &[u8]) -> u16 {
        const T: [u16; 16] = [
            0x0000, 0xCC01, 0xD801, 0x1400, 0xF001, 0x3C00, 0x2800, 0xE401, 0xA001, 0x6C00, 0x7800,
            0xB401, 0x5000, 0x9C01, 0x8801, 0x4400,
        ];
        let mut crc = 0u16;
        for &b in data {
            for nibble in [b & 0xF, b >> 4] {
                let tmp = T[(crc & 0xF) as usize];
                crc = ((crc >> 4) & 0x0FFF) ^ tmp ^ T[nibble as usize];
            }
        }
        crc
    }

    /// Parametri di una sessione sintetica (valori in unità FIT già scalate).
    #[derive(Clone)]
    struct Spec {
        start: i64,
        sport: u8,
        sub_sport: u8,
        elapsed_ms: u32,
        timer_ms: u32,
        distance_cm: u32,
        calories: u16,
        avg_hr: u8,
        max_hr: u8,
        avg_power: u16,
        ascent: u16,
    }

    impl Spec {
        fn running(start: i64) -> Spec {
            Spec {
                start,
                sport: 1,
                sub_sport: 0,
                elapsed_ms: 3_600_000,
                timer_ms: 3_500_000,
                distance_cm: 1_000_000,
                calories: 700,
                avg_hr: 150,
                max_hr: 170,
                avg_power: 250,
                ascent: 100,
            }
        }
    }

    /// Definizione (messaggio locale 0) e dati di un messaggio `session` (18).
    fn session_message(defined: bool, s: &Spec) -> Vec<u8> {
        let mut m = Vec::new();
        if defined {
            m.extend([0x40, 0, 0, 18, 0, 12]);
            // (numero campo, dimensione, tipo base)
            for f in [
                (253u8, 4u8, 0x86u8),
                (2, 4, 0x86),
                (5, 1, 0x00),
                (6, 1, 0x00),
                (7, 4, 0x86),
                (8, 4, 0x86),
                (9, 4, 0x86),
                (11, 2, 0x84),
                (16, 1, 0x02),
                (17, 1, 0x02),
                (20, 2, 0x84),
                (22, 2, 0x84),
            ] {
                m.extend([f.0, f.1, f.2]);
            }
        }
        m.push(0x00);
        let fit_start = (s.start - FIT_EPOCH) as u32;
        m.extend((fit_start + s.elapsed_ms / 1000).to_le_bytes());
        m.extend(fit_start.to_le_bytes());
        m.extend([s.sport, s.sub_sport]);
        m.extend(s.elapsed_ms.to_le_bytes());
        m.extend(s.timer_ms.to_le_bytes());
        m.extend(s.distance_cm.to_le_bytes());
        m.extend(s.calories.to_le_bytes());
        m.extend([s.avg_hr, s.max_hr]);
        m.extend(s.avg_power.to_le_bytes());
        m.extend(s.ascent.to_le_bytes());
        m
    }

    /// Messaggio `activity` (34, locale 1) con `timestamp` e `local_timestamp`.
    fn activity_message(timestamp: i64, local_fit_seconds: u32) -> Vec<u8> {
        let mut m = vec![0x41, 0, 0, 34, 0, 2, 253, 4, 0x86, 5, 4, 0x86, 0x01];
        m.extend(((timestamp - FIT_EPOCH) as u32).to_le_bytes());
        m.extend(local_fit_seconds.to_le_bytes());
        m
    }

    /// Assembla un file FIT valido (header e CRC compresi) dai messaggi dati.
    fn fit_file(messages: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = messages.concat();
        let mut file = vec![14, 0x20, 0x6C, 0x08];
        file.extend((body.len() as u32).to_le_bytes());
        file.extend(*b".FIT");
        file.extend(crc16(&file).to_le_bytes());
        file.extend(&body);
        file.extend(crc16(&file).to_le_bytes());
        file
    }

    fn sessions_file(specs: &[Spec]) -> Vec<u8> {
        let messages: Vec<Vec<u8>> = specs
            .iter()
            .enumerate()
            .map(|(i, s)| session_message(i == 0, s))
            .collect();
        fit_file(&messages)
    }

    const T0: i64 = 1_767_000_000;

    #[test]
    fn fit_garbage_is_malformed() {
        let bytes = include_bytes!("../../tests/fixtures/malformed.fit");
        assert_eq!(parse(bytes).unwrap_err().code(), "PARSE_ERROR");
    }

    #[test]
    fn tiny_input_is_malformed_or_empty() {
        assert_eq!(parse(&[]).unwrap_err().code(), "EMPTY_ACTIVITY");
        assert_eq!(parse(&[14, 0x20]).unwrap_err().code(), "PARSE_ERROR");
    }

    #[test]
    fn single_session_maps_every_field() {
        let a = parse(&sessions_file(&[Spec::running(T0)])).unwrap();
        assert_eq!(a.sport, Sport::Running);
        assert_eq!(a.sport_raw, "running/generic");
        assert_eq!(a.start_time.timestamp(), T0);
        assert_eq!(a.elapsed_s, 3600);
        assert_eq!(a.moving_s, Some(3500));
        assert_eq!(a.distance_m, Some(10_000.0));
        assert_eq!(a.elevation_gain_m, Some(100.0));
        assert_eq!(a.avg_hr, Some(150));
        assert_eq!(a.max_hr, Some(170));
        assert_eq!(a.avg_power_w, Some(250));
        assert_eq!(a.calories, Some(700));
        let speed = a.avg_speed_mps.unwrap();
        assert!((speed - 10_000.0 / 3500.0).abs() < 1e-9, "speed = {speed}");
        assert_eq!(a.utc_offset_seconds, None);
    }

    #[test]
    fn multiple_sessions_are_combined() {
        let bike = Spec {
            sport: 2,
            elapsed_ms: 1_000_000,
            timer_ms: 900_000,
            distance_cm: 1_000_000,
            avg_hr: 100,
            max_hr: 120,
            calories: 300,
            ascent: 50,
            ..Spec::running(T0)
        };
        let run = Spec {
            elapsed_ms: 3_000_000,
            timer_ms: 2_000_000,
            distance_cm: 500_000,
            avg_hr: 160,
            max_hr: 185,
            calories: 400,
            ascent: 20,
            ..Spec::running(T0 + 2000)
        };
        let a = parse(&sessions_file(&[bike, run])).unwrap();
        assert_eq!(a.sport, Sport::Running); // sessione con più tempo trascorso
        assert_eq!(a.start_time.timestamp(), T0);
        assert_eq!(a.elapsed_s, 4000);
        assert_eq!(a.moving_s, Some(2900));
        assert_eq!(a.distance_m, Some(15_000.0));
        assert_eq!(a.elevation_gain_m, Some(70.0));
        assert_eq!(a.calories, Some(700));
        assert_eq!(a.max_hr, Some(185));
        // (100 * 900 + 160 * 2000) / 2900 = 141.38
        assert_eq!(a.avg_hr, Some(141));
    }

    #[test]
    fn sport_mapping() {
        let sport_of = |sport: u8, sub_sport: u8| {
            let spec = Spec {
                sport,
                sub_sport,
                ..Spec::running(T0)
            };
            parse(&sessions_file(&[spec])).unwrap()
        };
        assert_eq!(sport_of(2, 0).sport, Sport::Cycling);
        assert_eq!(sport_of(5, 0).sport, Sport::Swimming);
        assert_eq!(sport_of(11, 0).sport, Sport::Walking);
        assert_eq!(sport_of(17, 0).sport, Sport::Hiking);
        let strength = sport_of(10, 20);
        assert_eq!(strength.sport, Sport::Strength);
        assert_eq!(strength.sport_raw, "training/strength_training");
        assert_eq!(sport_of(10, 0).sport, Sport::Other);
        assert_eq!(sport_of(254, 0).sport, Sport::Other);
    }

    #[test]
    fn zero_timer_time_does_not_divide_by_zero() {
        let spec = Spec {
            timer_ms: 0,
            ..Spec::running(T0)
        };
        let a = parse(&sessions_file(&[spec])).unwrap();
        assert_eq!(a.moving_s, Some(0));
        assert_eq!(a.avg_speed_mps, None);
        assert_eq!(a.avg_hr, Some(150));
    }

    #[test]
    fn file_without_session_is_empty() {
        let file = fit_file(&[activity_message(T0, (T0 - FIT_EPOCH) as u32)]);
        assert_eq!(parse(&file).unwrap_err().code(), "EMPTY_ACTIVITY");
    }

    #[test]
    fn utc_offset_from_activity_message() {
        let local = (T0 - FIT_EPOCH + 7200) as u32;
        let file = fit_file(&[
            session_message(true, &Spec::running(T0)),
            activity_message(T0, local),
        ]);
        assert_eq!(parse(&file).unwrap().utc_offset_seconds, Some(7200));
    }

    #[test]
    fn implausible_local_timestamp_is_ignored() {
        // Alcuni dispositivi scrivono l'epoca FIT (0) come ora locale
        let file = fit_file(&[
            session_message(true, &Spec::running(T0)),
            activity_message(T0, 0),
        ]);
        assert_eq!(parse(&file).unwrap().utc_offset_seconds, None);
    }

    #[test]
    fn truncated_and_corrupted_files_never_panic() {
        let file = fit_file(&[
            session_message(true, &Spec::running(T0)),
            activity_message(T0, (T0 - FIT_EPOCH) as u32),
        ]);
        // `collect` non ha il catch_unwind di `parse`: un panic farebbe fallire il test
        for len in 0..file.len() {
            let _ = collect(&file[..len]);
        }
        for i in 0..file.len() {
            for mask in [0x01u8, 0x80, 0xFF] {
                let mut copy = file.clone();
                copy[i] ^= mask;
                let _ = collect(&copy);
                let _ = parse(&copy);
            }
        }
    }

    #[test]
    fn non_finite_and_negative_values_are_absent() {
        assert_eq!(number(&Value::Float64(f64::NAN)), None);
        assert_eq!(number(&Value::Float32(f32::INFINITY)), None);
        assert_eq!(non_negative(&Value::SInt16(-5)), None);
        assert_eq!(non_negative(&Value::Invalid), None);
        assert_eq!(non_negative(&Value::String("12".into())), None);
        assert_eq!(non_negative(&Value::UInt8(7)), Some(7.0));
    }

    #[test]
    fn huge_durations_saturate() {
        // 600 sessioni da ~4 milioni di secondi: la somma supera i32::MAX
        let spec = Spec {
            elapsed_ms: 4_000_000_000,
            timer_ms: 4_000_000_000,
            ..Spec::running(T0)
        };
        let a = parse(&sessions_file(&vec![spec; 600])).unwrap();
        assert_eq!(a.elapsed_s, i32::MAX);
        assert_eq!(a.moving_s, Some(i32::MAX));
    }
}
