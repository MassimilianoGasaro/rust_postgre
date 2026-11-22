//! tcx — Parsing TCX
//!
//! Lettura in streaming con `quick_xml::Reader` della prima `<Activity>`.
//! Gli elementi sono riconosciuti dal local name (qualsiasi prefisso di
//! namespace) e dal padre, per distinguere ad es. il `Cadence` del lap da
//! quello del trackpoint.

use chrono::{DateTime, Utc};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::metrics::elevation_gain;
use super::{
    check_depth, finite_number, local_name, parse_time, rounded_mean, seconds_to_i32, ParseError,
    ParsedActivity, Sport,
};

/// Dati di un giro (`<Lap>`).
#[derive(Default)]
struct Lap {
    total_time_s: f64,
    distance_m: Option<f64>,
    calories: Option<f64>,
    avg_hr: Option<f64>,
    max_hr: Option<f64>,
    cadence: Option<f64>,
}

/// Stato della lettura: pila dei local name aperti e dati raccolti.
#[derive(Default)]
struct TcxReader {
    stack: Vec<String>,
    /// Quanti `<Trackpoint>` sono aperti (evita di scorrere la pila a ogni testo)
    trackpoint_depth: usize,
    activity_seen: bool,
    activity_done: bool,
    sport_raw: String,
    id: Option<DateTime<Utc>>,
    laps: Vec<Lap>,
    current_lap: Option<Lap>,
    times: Vec<DateTime<Utc>>,
    altitudes: Vec<f64>,
    watts: Vec<f64>,
    run_cadences: Vec<f64>,
}

impl TcxReader {
    fn start(&mut self, e: &BytesStart<'_>) -> Result<(), ParseError> {
        let name = local_name(e.local_name());
        if self.stack.is_empty() && name != "TrainingCenterDatabase" {
            return Err(ParseError::Malformed(
                "root element is not <TrainingCenterDatabase>".into(),
            ));
        }
        check_depth(self.stack.len())?;
        match name.as_str() {
            "Activity" if !self.activity_seen => {
                self.activity_seen = true;
                for attr in e.attributes() {
                    let attr = attr.map_err(|e| ParseError::Malformed(e.to_string()))?;
                    if local_name(attr.key.local_name()) == "Sport" {
                        self.sport_raw = attr
                            .unescape_value()
                            .map_err(|e| ParseError::Malformed(e.to_string()))?
                            .into_owned();
                    }
                }
            }
            "Lap" if self.activity_seen => self.current_lap = Some(Lap::default()),
            "Trackpoint" => self.trackpoint_depth += 1,
            _ => {}
        }
        self.stack.push(name);
        Ok(())
    }

    fn text(&mut self, text: &str) -> Result<(), ParseError> {
        let text = text.trim();
        if text.is_empty() || !self.activity_seen {
            return Ok(());
        }
        let n = self.stack.len();
        let Some(name) = self.stack.last().map(String::as_str) else {
            return Ok(());
        };
        let parent = n.checked_sub(2).map(|i| self.stack[i].as_str());
        let number = finite_number(text);
        let in_trackpoint = self.trackpoint_depth > 0;

        match (parent, name) {
            (Some("Activity"), "Id") => self.id = Some(parse_time(text)?),
            (Some("Trackpoint"), "Time") => self.times.push(parse_time(text)?),
            (Some("Trackpoint"), "AltitudeMeters") => self.altitudes.extend(number),
            (_, "Watts") if in_trackpoint => self.watts.extend(number),
            (_, "RunCadence") if in_trackpoint => self.run_cadences.extend(number),
            (Some("Lap"), field) => {
                if let Some(lap) = self.current_lap.as_mut() {
                    match field {
                        "TotalTimeSeconds" => lap.total_time_s = number.unwrap_or(0.0).max(0.0),
                        "DistanceMeters" => lap.distance_m = number,
                        "Calories" => lap.calories = number,
                        "Cadence" => lap.cadence = number,
                        _ => {}
                    }
                }
            }
            (Some("AverageHeartRateBpm"), "Value") => {
                if let Some(lap) = self.current_lap.as_mut() {
                    lap.avg_hr = number;
                }
            }
            (Some("MaximumHeartRateBpm"), "Value") => {
                if let Some(lap) = self.current_lap.as_mut() {
                    lap.max_hr = number;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn end(&mut self) {
        match self.stack.pop().as_deref() {
            Some("Lap") => {
                if let Some(lap) = self.current_lap.take() {
                    self.laps.push(lap);
                }
            }
            Some("Trackpoint") => self.trackpoint_depth = self.trackpoint_depth.saturating_sub(1),
            // Solo la prima attività viene letta
            Some("Activity") if self.activity_seen => self.activity_done = true,
            _ => {}
        }
    }
}

/// Sport normalizzato a partire dall'attributo `Sport` di `<Activity>`.
fn sport_from_attr(raw: &str) -> Sport {
    match raw {
        "Running" => Sport::Running,
        "Biking" => Sport::Cycling,
        _ => Sport::Other,
    }
}

/// Media pesata; se i pesi sono tutti nulli ripiega sulla media semplice.
fn weighted_mean(values: &[(f64, f64)]) -> Option<f64> {
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

/// Legge un file TCX e ne calcola le metriche.
pub fn parse(bytes: &[u8]) -> Result<ParsedActivity, ParseError> {
    let mut reader = Reader::from_reader(bytes);
    let mut buf = Vec::new();
    let mut state = TcxReader::default();
    let mut seen_root = false;

    while !state.activity_done {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => {
                state.start(&e)?;
                seen_root = true;
            }
            Event::Empty(e) => {
                state.start(&e)?;
                seen_root = true;
                state.end();
            }
            Event::End(_) => state.end(),
            Event::Text(t) => {
                let text = t.unescape().map_err(|e| ParseError::Malformed(e.to_string()))?;
                state.text(&text)?;
            }
            Event::CData(c) => state.text(&String::from_utf8_lossy(&c))?,
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    if !seen_root {
        return Err(ParseError::Malformed(
            "root element <TrainingCenterDatabase> not found".into(),
        ));
    }
    // Il Reader grezzo non segnala elementi rimasti aperti a fine file:
    // il documento deve essere completo, oppure la prima attività chiusa
    if !state.activity_done && !state.stack.is_empty() {
        return Err(ParseError::Malformed("unexpected end of file".into()));
    }
    build(state)
}

/// Calcola le metriche aggregate dai dati raccolti.
fn build(state: TcxReader) -> Result<ParsedActivity, ParseError> {
    if !state.activity_seen || (state.laps.is_empty() && state.times.is_empty()) {
        return Err(ParseError::Empty);
    }
    let Some(start_time) = state.id.or_else(|| state.times.first().copied()) else {
        return Err(ParseError::Empty);
    };

    let laps = &state.laps;
    let total_time_s: f64 = laps.iter().map(|l| l.total_time_s).sum();
    let moving_s = (!laps.is_empty()).then(|| seconds_to_i32(total_time_s));
    let elapsed_s = match state.times.last() {
        Some(last) => i32::try_from((*last - start_time).num_seconds().max(0)).unwrap_or(i32::MAX),
        None => seconds_to_i32(total_time_s),
    };

    let distances: Vec<f64> = laps.iter().filter_map(|l| l.distance_m).collect();
    let distance_m = (!distances.is_empty()).then(|| distances.iter().sum::<f64>());
    let calories: Vec<f64> = laps.iter().filter_map(|l| l.calories).collect();
    let calories = (!calories.is_empty()).then(|| calories.iter().sum::<f64>().round() as i32);

    let weighted = |pick: fn(&Lap) -> Option<f64>| -> Option<i32> {
        let values: Vec<(f64, f64)> = laps
            .iter()
            .filter_map(|l| pick(l).map(|v| (v, l.total_time_s)))
            .collect();
        weighted_mean(&values).map(|v| v.round() as i32)
    };
    let avg_cadence = weighted(|l| l.cadence).or_else(|| rounded_mean(&state.run_cadences));
    let max_hr = laps
        .iter()
        .filter_map(|l| l.max_hr)
        .reduce(f64::max)
        .map(|h| h.round() as i32);

    Ok(ParsedActivity {
        sport: sport_from_attr(&state.sport_raw),
        sport_raw: state.sport_raw,
        name: None,
        start_time,
        utc_offset_seconds: None,
        elapsed_s,
        moving_s,
        distance_m,
        elevation_gain_m: (!state.altitudes.is_empty()).then(|| elevation_gain(&state.altitudes)),
        avg_hr: weighted(|l| l.avg_hr),
        max_hr,
        avg_speed_mps: match (distance_m, moving_s) {
            (Some(d), Some(m)) if m > 0 => Some(d / m as f64),
            _ => None,
        },
        avg_power_w: rounded_mean(&state.watts),
        avg_cadence,
        calories,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn parse_ride_tcx() {
        let a = parse(include_bytes!("../../tests/fixtures/ride.tcx")).unwrap();
        assert_eq!(a.sport, Sport::Cycling);
        assert_eq!(a.sport_raw, "Biking");
        assert_eq!(a.name, None);
        assert_eq!(a.start_time, Utc.with_ymd_and_hms(2026, 1, 10, 8, 0, 0).unwrap());
        assert_eq!(a.utc_offset_seconds, None);
        assert_eq!(a.elapsed_s, 900);
        assert_eq!(a.moving_s, Some(900));
        assert_eq!(a.distance_m, Some(7500.0));
        assert_eq!(a.calories, Some(160));
        assert_eq!(a.avg_hr, Some(140));
        assert_eq!(a.max_hr, Some(175));
        assert_eq!(a.avg_power_w, Some(200));
        assert_eq!(a.avg_cadence, Some(83));
        assert_eq!(a.elevation_gain_m, Some(24.0));
        let v = a.avg_speed_mps.unwrap();
        assert!((8.33..8.34).contains(&v), "v = {v}");
    }

    #[test]
    fn run_cadence_is_used_when_laps_have_none() {
        let xml = br#"<TrainingCenterDatabase xmlns:x="u"><Activities><Activity Sport="Running">
            <Id>2026-01-10T08:00:00Z</Id>
            <Lap><TotalTimeSeconds>60</TotalTimeSeconds><DistanceMeters>200</DistanceMeters>
              <Track>
                <Trackpoint><Time>2026-01-10T08:00:00Z</Time><Extensions><x:TPX><x:RunCadence>80</x:RunCadence></x:TPX></Extensions></Trackpoint>
                <Trackpoint><Time>2026-01-10T08:01:00Z</Time><Extensions><x:TPX><x:RunCadence>90</x:RunCadence></x:TPX></Extensions></Trackpoint>
              </Track></Lap>
            </Activity></Activities></TrainingCenterDatabase>"#;
        let a = parse(xml).unwrap();
        assert_eq!(a.sport, Sport::Running);
        assert_eq!(a.avg_cadence, Some(85));
        assert_eq!(a.avg_hr, None);
        assert_eq!(a.avg_power_w, None);
        assert_eq!(a.elevation_gain_m, None);
        assert_eq!(a.calories, None);
    }

    #[test]
    fn unknown_sport_is_other() {
        let xml = br#"<TrainingCenterDatabase><Activities><Activity Sport="Other">
            <Id>2026-01-10T08:00:00Z</Id>
            <Lap><TotalTimeSeconds>60</TotalTimeSeconds></Lap>
            </Activity></Activities></TrainingCenterDatabase>"#;
        let a = parse(xml).unwrap();
        assert_eq!(a.sport, Sport::Other);
        assert_eq!(a.sport_raw, "Other");
        assert_eq!(a.elapsed_s, 60);
        assert_eq!(a.distance_m, None);
        assert_eq!(a.avg_speed_mps, None);
    }

    #[test]
    fn only_first_activity_is_read() {
        let xml = br#"<TrainingCenterDatabase><Activities>
            <Activity Sport="Running"><Id>2026-01-10T08:00:00Z</Id>
              <Lap><TotalTimeSeconds>60</TotalTimeSeconds><DistanceMeters>100</DistanceMeters></Lap></Activity>
            <Activity Sport="Biking"><Id>2026-01-11T08:00:00Z</Id>
              <Lap><TotalTimeSeconds>999</TotalTimeSeconds><DistanceMeters>9999</DistanceMeters></Lap></Activity>
            </Activities></TrainingCenterDatabase>"#;
        let a = parse(xml).unwrap();
        assert_eq!(a.sport, Sport::Running);
        assert_eq!(a.distance_m, Some(100.0));
    }

    #[test]
    fn tcx_too_deep_is_malformed() {
        let xml = format!(
            "<TrainingCenterDatabase>{}{}</TrainingCenterDatabase>",
            "<a>".repeat(100),
            "</a>".repeat(100)
        );
        assert_eq!(parse(xml.as_bytes()).unwrap_err().code(), "PARSE_ERROR");
    }

    #[test]
    fn truncated_tcx_is_malformed() {
        let full = include_bytes!("../../tests/fixtures/ride.tcx");
        // Taglio arbitrario (anche in mezzo a un tag)
        assert_eq!(parse(&full[..full.len() * 2 / 3]).unwrap_err().code(), "PARSE_ERROR");
        // Taglio pulito subito dopo un elemento chiuso
        let text = std::str::from_utf8(full).unwrap();
        let end = text.find("</Trackpoint>").unwrap() + "</Trackpoint>".len();
        let err = parse(&text.as_bytes()[..end]).unwrap_err();
        assert_eq!(err.code(), "PARSE_ERROR");
        assert!(err.to_string().contains("unexpected end of file"), "{err}");
    }

    #[test]
    fn tcx_non_finite_values_are_ignored() {
        let xml = br#"<TrainingCenterDatabase><Activities><Activity Sport="Biking">
            <Id>2026-01-10T08:00:00Z</Id>
            <Lap><TotalTimeSeconds>NaN</TotalTimeSeconds><DistanceMeters>inf</DistanceMeters>
              <Calories>NaN</Calories><Track>
                <Trackpoint><Time>2026-01-10T08:00:00Z</Time><AltitudeMeters>NaN</AltitudeMeters></Trackpoint>
              </Track></Lap>
            </Activity></Activities></TrainingCenterDatabase>"#;
        let a = parse(xml).unwrap();
        assert_eq!(a.distance_m, None);
        assert_eq!(a.calories, None);
        assert_eq!(a.elevation_gain_m, None);
        assert_eq!(a.moving_s, Some(0));
    }

    #[test]
    fn tcx_without_activity_is_empty() {
        let xml = br#"<TrainingCenterDatabase><Activities></Activities></TrainingCenterDatabase>"#;
        assert_eq!(parse(xml).unwrap_err().code(), "EMPTY_ACTIVITY");
    }

    #[test]
    fn tcx_invalid_id_is_malformed() {
        let xml = br#"<TrainingCenterDatabase><Activities><Activity Sport="Running"><Id>yesterday</Id></Activity></Activities></TrainingCenterDatabase>"#;
        assert_eq!(parse(xml).unwrap_err().code(), "PARSE_ERROR");
    }
}
