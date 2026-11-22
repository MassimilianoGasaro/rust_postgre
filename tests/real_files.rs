//! Test sui file reali del proprietario (`docs/files/`, non versionati).
//!
//! Sono `#[ignore]`: si eseguono a mano con
//! `cargo test --test real_files -- --ignored`.

use std::io::Read;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use rust_postgre::parsing::{FileFormat, ParsedActivity, Sport, parse};

/// Legge un file da `docs/files/`; se manca il test fallisce con un messaggio chiaro.
fn read_real(name: &str) -> Vec<u8> {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "docs", "files", name]
        .iter()
        .collect();
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "file reale mancante: {} ({e}). Copiarlo in docs/files/ per eseguire questo test.",
            path.display()
        )
    })
}

fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .expect("timestamp di test valido")
        .with_timezone(&Utc)
}

/// Estrae dallo ZIP la prima voce `.fit`.
fn fit_from_zip(name: &str) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(read_real(name)))
        .expect("archivio ZIP leggibile");
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).expect("voce ZIP leggibile");
        if entry.name().to_ascii_lowercase().ends_with(".fit") {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).expect("voce FIT leggibile");
            return buf;
        }
    }
    panic!("nessun file .fit dentro {name}");
}

fn parse_ok(format: FileFormat, bytes: &[u8]) -> ParsedActivity {
    parse(format, bytes).unwrap_or_else(|e| panic!("parsing fallito: {e}"))
}

#[test]
#[ignore]
fn real_coros_fit_parses() {
    let a = parse_ok(
        FileFormat::Fit,
        &read_real("481934560924303460_f9d4fbc5e2.fit"),
    );
    println!(
        "coros: sport={:?} raw={} distance_m={:?} elapsed_s={} moving_s={:?} start={}",
        a.sport, a.sport_raw, a.distance_m, a.elapsed_s, a.moving_s, a.start_time
    );
    assert!(a.elapsed_s > 0);
    assert!(a.distance_m.is_some_and(|d| d > 0.0));
    assert!(a.start_time > utc("2015-01-01T00:00:00Z"));
}

#[test]
#[ignore]
fn real_garmin_tcx_parses() {
    let a = parse_ok(FileFormat::Tcx, &read_real("activity_25087995114.tcx"));
    assert_eq!(a.sport, Sport::Cycling);
    assert_eq!(a.start_time, utc("2026-11-15T11:08:59Z"));
}

#[test]
#[ignore]
fn real_garmin_zip_fit_matches_tcx() {
    let fit = parse_ok(FileFormat::Fit, &fit_from_zip("25143684168.zip"));
    let tcx = parse_ok(FileFormat::Tcx, &read_real("activity_25143684168.tcx"));
    println!(
        "garmin: fit sport={:?} start={} dist={:?} | tcx sport={:?} start={} dist={:?}",
        fit.sport, fit.start_time, fit.distance_m, tcx.sport, tcx.start_time, tcx.distance_m
    );
    assert_eq!(fit.sport, Sport::Cycling);
    assert_eq!(tcx.sport, Sport::Cycling);
    let delta = (fit.start_time - tcx.start_time).num_seconds().abs();
    assert!(delta <= 60, "inizio FIT/TCX distano {delta} s");
    let (df, dt) = (
        fit.distance_m.expect("distanza FIT"),
        tcx.distance_m.expect("distanza TCX"),
    );
    assert!(
        dt > 0.0 && (df - dt).abs() / dt <= 0.02,
        "distanze: fit {df} tcx {dt}"
    );
}
