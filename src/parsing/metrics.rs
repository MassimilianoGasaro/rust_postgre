//! metrics — Calcoli puri sulle tracce (dislivello, distanza, tempo in movimento)

use chrono::{DateTime, Utc};

/// Soglia di isteresi per il dislivello positivo (metri).
const ELEVATION_HYSTERESIS_M: f64 = 2.0;
/// Raggio medio terrestre (metri).
const EARTH_RADIUS_M: f64 = 6_371_000.0;
/// Velocità minima per considerare un intervallo "in movimento" (m/s).
const MOVING_SPEED_MPS: f64 = 0.5;

/// Dislivello positivo con isteresi di 2 m: una salita conta solo quando
/// supera di oltre 2 m la quota di riferimento, per filtrare il rumore.
pub fn elevation_gain(altitudes: &[f64]) -> f64 {
    let Some(&first) = altitudes.first() else {
        return 0.0;
    };
    let mut reference = first;
    let mut gain = 0.0;
    for &a in altitudes {
        if a > reference + ELEVATION_HYSTERESIS_M {
            gain += a - reference;
            reference = a;
        } else if a < reference {
            reference = a;
        }
    }
    gain
}

/// Distanza in metri tra due punti (lat, lon in gradi) — formula dell'haversine.
pub fn haversine_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (lat1, lon1) = (a.0.to_radians(), a.1.to_radians());
    let (lat2, lon2) = (b.0.to_radians(), b.1.to_radians());
    let h = ((lat2 - lat1) / 2.0).sin().powi(2)
        + lat1.cos() * lat2.cos() * ((lon2 - lon1) / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * h.sqrt().asin()
}

/// Secondi in movimento: somma degli intervalli tra campioni consecutivi con
/// velocità > 0,5 m/s. Ogni campione è (istante, distanza cumulata in metri).
pub fn moving_seconds(samples: &[(DateTime<Utc>, f64)]) -> i32 {
    let moving_ms: i64 = samples
        .windows(2)
        .filter_map(|w| {
            let dt_ms = (w[1].0 - w[0].0).num_milliseconds();
            if dt_ms <= 0 {
                return None;
            }
            let speed = (w[1].1 - w[0].1) / (dt_ms as f64 / 1000.0);
            (speed > MOVING_SPEED_MPS).then_some(dt_ms)
        })
        .sum();
    (moving_ms as f64 / 1000.0).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn elevation_gain_uses_hysteresis() {
        let alts = [100., 101., 103., 102., 106., 106., 104., 110., 111., 111., 112.];
        assert_eq!(elevation_gain(&alts), 13.0);
    }

    #[test]
    fn elevation_gain_ignores_noise() {
        assert_eq!(elevation_gain(&[100., 101., 100., 101.]), 0.0);
        assert_eq!(elevation_gain(&[]), 0.0);
    }

    #[test]
    fn haversine_one_millidegree_of_latitude() {
        let d = haversine_m((0.0, 0.0), (0.001, 0.0));
        assert!((111.0..111.4).contains(&d), "d = {d}");
    }

    #[test]
    fn moving_seconds_skips_stopped_intervals() {
        let t0 = Utc.with_ymd_and_hms(2026, 1, 10, 7, 0, 0).unwrap();
        let at = |s: i64, d: f64| (t0 + chrono::Duration::seconds(s), d);
        let samples = [at(0, 0.0), at(10, 50.0), at(20, 50.0), at(30, 100.0)];
        assert_eq!(moving_seconds(&samples), 20);
    }
}
