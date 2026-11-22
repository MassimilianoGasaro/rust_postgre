//! gpx — Parsing GPX
//!
//! Lettura in streaming con `quick_xml::Reader`: gli elementi vengono
//! riconosciuti dal local name, quindi i prefissi di namespace (es.
//! `gpxtpx:hr`) sono irrilevanti.

use chrono::{DateTime, Utc};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use super::metrics::{elevation_gain, haversine_m, moving_seconds};
use super::{
    ParseError, ParsedActivity, Sport, check_depth, finite_number, local_name, parse_time,
    rounded_mean,
};

/// Punto di traccia con tempo (i punti senza tempo vengono scartati).
struct Point {
    lat: f64,
    lon: f64,
    time: Option<DateTime<Utc>>,
    ele: Option<f64>,
    hr: Option<f64>,
    cad: Option<f64>,
    power: Option<f64>,
}

/// Stato della lettura: pila dei local name aperti e dati raccolti.
#[derive(Default)]
struct GpxReader {
    stack: Vec<String>,
    name: Option<String>,
    sport_raw: Option<String>,
    points: Vec<Point>,
    current: Option<Point>,
}

impl GpxReader {
    fn start(&mut self, e: &BytesStart<'_>) -> Result<(), ParseError> {
        let name = local_name(e.local_name());
        if self.stack.is_empty() && name != "gpx" {
            return Err(ParseError::Malformed("root element is not <gpx>".into()));
        }
        check_depth(self.stack.len())?;
        if name == "trkpt" {
            let (mut lat, mut lon) = (None, None);
            for attr in e.attributes() {
                let attr = attr.map_err(|e| ParseError::Malformed(e.to_string()))?;
                let value = attr
                    .unescape_value()
                    .map_err(|e| ParseError::Malformed(e.to_string()))?;
                match local_name(attr.key.local_name()).as_str() {
                    "lat" => lat = finite_number(&value).filter(|v| v.abs() <= 90.0),
                    "lon" => lon = finite_number(&value).filter(|v| v.abs() <= 180.0),
                    _ => {}
                }
            }
            let (Some(lat), Some(lon)) = (lat, lon) else {
                return Err(ParseError::Malformed("invalid trkpt coordinates".into()));
            };
            self.current = Some(Point {
                lat,
                lon,
                time: None,
                ele: None,
                hr: None,
                cad: None,
                power: None,
            });
        }
        self.stack.push(name);
        Ok(())
    }

    fn text(&mut self, text: &str) -> Result<(), ParseError> {
        let text = text.trim();
        let Some(name) = self.stack.last() else {
            return Ok(());
        };
        if let Some(point) = self.current.as_mut() {
            match name.as_str() {
                "time" => point.time = Some(parse_time(text)?),
                "ele" => point.ele = finite_number(text),
                "hr" => point.hr = finite_number(text),
                "cad" => point.cad = finite_number(text),
                "power" => point.power = finite_number(text),
                _ => {}
            }
            return Ok(());
        }
        let parent = self
            .stack
            .len()
            .checked_sub(2)
            .map(|i| self.stack[i].as_str());
        if parent == Some("trk") && !text.is_empty() {
            match name.as_str() {
                "name" if self.name.is_none() => self.name = Some(text.to_owned()),
                "type" if self.sport_raw.is_none() => self.sport_raw = Some(text.to_owned()),
                _ => {}
            }
        }
        Ok(())
    }

    fn end(&mut self) {
        if self.stack.pop().as_deref() == Some("trkpt")
            && let Some(point) = self.current.take()
            && point.time.is_some()
        {
            self.points.push(point);
        }
    }
}

/// Sport normalizzato a partire dal `<type>` della traccia.
pub fn sport_from_type(raw: &str) -> Sport {
    let t = raw.to_lowercase();
    if t.contains("run") {
        Sport::Running
    } else if t.contains("bik") || t.contains("cycl") || t.contains("ride") {
        Sport::Cycling
    } else if t.contains("swim") {
        Sport::Swimming
    } else if t.contains("walk") {
        Sport::Walking
    } else if t.contains("hik") {
        Sport::Hiking
    } else if t.contains("strength") || t.contains("weight") {
        Sport::Strength
    } else {
        Sport::Other
    }
}

/// Legge un file GPX e ne calcola le metriche.
pub fn parse(bytes: &[u8]) -> Result<ParsedActivity, ParseError> {
    let mut reader = Reader::from_reader(bytes);
    let mut buf = Vec::new();
    let mut state = GpxReader::default();
    let mut seen_root = false;

    loop {
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
                let text = t
                    .unescape()
                    .map_err(|e| ParseError::Malformed(e.to_string()))?;
                state.text(&text)?;
            }
            Event::CData(c) => state.text(&String::from_utf8_lossy(&c))?,
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    if !seen_root {
        return Err(ParseError::Malformed("root element <gpx> not found".into()));
    }
    // Il Reader grezzo non segnala elementi rimasti aperti a fine file
    if !state.stack.is_empty() {
        return Err(ParseError::Malformed("unexpected end of file".into()));
    }
    build(state)
}

/// Calcola le metriche aggregate dai punti raccolti.
fn build(state: GpxReader) -> Result<ParsedActivity, ParseError> {
    let points = &state.points;
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return Err(ParseError::Empty);
    };
    let (Some(start_time), Some(end_time)) = (first.time, last.time) else {
        return Err(ParseError::Empty);
    };
    let elapsed_s = i32::try_from((end_time - start_time).num_seconds().max(0)).unwrap_or(i32::MAX);

    // Distanza cumulata punto per punto (serve anche al tempo in movimento)
    let mut cumulative = 0.0;
    let mut samples = Vec::with_capacity(points.len());
    let mut prev: Option<&Point> = None;
    for p in points {
        if let Some(q) = prev {
            cumulative += haversine_m((q.lat, q.lon), (p.lat, p.lon));
        }
        if let Some(time) = p.time {
            samples.push((time, cumulative));
        }
        prev = Some(p);
    }
    let moving_s = moving_seconds(&samples);

    let altitudes: Vec<f64> = points.iter().filter_map(|p| p.ele).collect();
    let hrs: Vec<f64> = points.iter().filter_map(|p| p.hr).collect();
    let cads: Vec<f64> = points.iter().filter_map(|p| p.cad).collect();
    let powers: Vec<f64> = points.iter().filter_map(|p| p.power).collect();
    let sport_raw = state.sport_raw.unwrap_or_default();

    Ok(ParsedActivity {
        sport: sport_from_type(&sport_raw),
        sport_raw,
        name: state.name,
        start_time,
        utc_offset_seconds: None,
        elapsed_s,
        moving_s: Some(moving_s),
        distance_m: Some(cumulative),
        elevation_gain_m: (!altitudes.is_empty()).then(|| elevation_gain(&altitudes)),
        avg_hr: rounded_mean(&hrs),
        max_hr: hrs
            .iter()
            .cloned()
            .reduce(f64::max)
            .map(|h| h.round() as i32),
        avg_speed_mps: (moving_s > 0).then(|| cumulative / moving_s as f64),
        avg_power_w: rounded_mean(&powers),
        avg_cadence: rounded_mean(&cads),
        calories: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/").to_owned() + name)
            .unwrap()
    }

    #[test]
    fn parse_run_gpx() {
        let a = parse(&fixture("run.gpx")).unwrap();
        assert_eq!(a.sport, Sport::Running);
        assert_eq!(a.sport_raw, "running");
        assert_eq!(a.name.as_deref(), Some("Morning Run"));
        assert_eq!(
            a.start_time,
            Utc.with_ymd_and_hms(2026, 1, 10, 7, 0, 0).unwrap()
        );
        assert_eq!(a.utc_offset_seconds, None);
        assert_eq!(a.elapsed_s, 100);
        assert_eq!(a.moving_s, Some(100));
        let d = a.distance_m.unwrap();
        assert!((1110.0..1114.0).contains(&d), "d = {d}");
        assert_eq!(a.elevation_gain_m, Some(13.0));
        assert_eq!(a.avg_hr, Some(145));
        assert_eq!(a.max_hr, Some(150));
        assert_eq!(a.avg_power_w, None);
        assert_eq!(a.avg_cadence, None);
        assert_eq!(a.calories, None);
        let v = a.avg_speed_mps.unwrap();
        assert!((11.0..11.2).contains(&v), "v = {v}");
    }

    #[test]
    fn parse_ride_same_as_tcx_gpx() {
        let a = parse(&fixture("ride_same_as_tcx.gpx")).unwrap();
        assert_eq!(a.sport, Sport::Cycling);
        assert_eq!(
            a.start_time,
            Utc.with_ymd_and_hms(2026, 1, 10, 8, 0, 30).unwrap()
        );
        assert_eq!(a.elapsed_s, 240);
        assert_eq!(a.avg_hr, None);
    }

    #[test]
    fn gpx_without_points_is_empty() {
        let xml = br#"<gpx version="1.1"><trk><name>x</name><trkseg></trkseg></trk></gpx>"#;
        assert_eq!(parse(xml).unwrap_err().code(), "EMPTY_ACTIVITY");
    }

    #[test]
    fn gpx_points_without_time_are_empty() {
        let xml = br#"<gpx><trk><trkseg><trkpt lat="0" lon="0"><ele>1</ele></trkpt></trkseg></trk></gpx>"#;
        assert_eq!(parse(xml).unwrap_err().code(), "EMPTY_ACTIVITY");
    }

    #[test]
    fn gpx_non_finite_or_out_of_range_coordinates_are_malformed() {
        for (lat, lon) in [
            ("NaN", "0"),
            ("inf", "0"),
            ("91", "0"),
            ("0", "181"),
            ("0", "-NaN"),
        ] {
            let xml = format!(
                r#"<gpx><trk><trkseg><trkpt lat="{lat}" lon="{lon}"><time>2026-01-10T07:00:00Z</time></trkpt></trkseg></trk></gpx>"#
            );
            assert_eq!(
                parse(xml.as_bytes()).unwrap_err().code(),
                "PARSE_ERROR",
                "{lat},{lon}"
            );
        }
    }

    #[test]
    fn gpx_non_finite_elevation_is_ignored() {
        let xml = br#"<gpx><trk><trkseg>
            <trkpt lat="0" lon="0"><ele>NaN</ele><time>2026-01-10T07:00:00Z</time></trkpt>
            <trkpt lat="0.001" lon="0"><ele>100</ele><time>2026-01-10T07:00:10Z</time></trkpt>
            <trkpt lat="0.002" lon="0"><ele>110</ele><time>2026-01-10T07:00:20Z</time></trkpt>
            </trkseg></trk></gpx>"#;
        let a = parse(xml).unwrap();
        assert_eq!(a.elevation_gain_m, Some(10.0));
    }

    #[test]
    fn gpx_too_deep_is_malformed() {
        let xml = format!("<gpx>{}{}</gpx>", "<a>".repeat(100), "</a>".repeat(100));
        assert_eq!(parse(xml.as_bytes()).unwrap_err().code(), "PARSE_ERROR");
    }

    #[test]
    fn truncated_gpx_is_malformed() {
        let full = fixture("run.gpx");
        // Taglio arbitrario (anche in mezzo a un tag)
        assert_eq!(
            parse(&full[..full.len() * 2 / 3]).unwrap_err().code(),
            "PARSE_ERROR"
        );
        // Taglio pulito subito dopo un elemento chiuso
        let text = String::from_utf8(full).unwrap();
        let end = text.find("</trkpt>").unwrap() + "</trkpt>".len();
        let err = parse(&text.as_bytes()[..end]).unwrap_err();
        assert_eq!(err.code(), "PARSE_ERROR");
        assert!(err.to_string().contains("unexpected end of file"), "{err}");
    }

    #[test]
    fn gpx_invalid_coordinates_are_malformed() {
        let xml = br#"<gpx><trk><trkseg><trkpt lat="abc" lon="0"><time>2026-01-10T07:00:00Z</time></trkpt></trkseg></trk></gpx>"#;
        assert_eq!(parse(xml).unwrap_err().code(), "PARSE_ERROR");
    }

    #[test]
    fn sport_mapping_from_type() {
        for (raw, sport) in [
            ("running", Sport::Running),
            ("trail_run", Sport::Running),
            ("biking", Sport::Cycling),
            ("cycling", Sport::Cycling),
            ("virtual_ride", Sport::Cycling),
            ("swimming", Sport::Swimming),
            ("walking", Sport::Walking),
            ("hiking", Sport::Hiking),
            ("strength_training", Sport::Strength),
            ("weight_training", Sport::Strength),
            ("rowing", Sport::Other),
            ("", Sport::Other),
        ] {
            assert_eq!(sport_from_type(raw), sport, "{raw}");
        }
    }
}
