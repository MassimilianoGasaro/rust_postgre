mod common;

use axum::http::{Method, StatusCode};
use chrono::{DateTime, Utc};
use rust_postgre::parsing::{FileFormat, ParsedActivity, Sport};
use rust_postgre::repositories::activity_repository;
use serde_json::{Value, json};
use sqlx::PgPool;

use common::TestApp;

/// Attività di test con le metriche indicate (`elapsed_s` sempre 1000).
fn parsed(
    sport: Sport,
    start: &str,
    moving_s: Option<i32>,
    distance_m: Option<f64>,
    elevation_gain_m: Option<f64>,
) -> ParsedActivity {
    ParsedActivity {
        sport,
        sport_raw: sport.as_str().to_string(),
        name: None,
        start_time: start.parse::<DateTime<Utc>>().unwrap(),
        utc_offset_seconds: None,
        elapsed_s: 1000,
        moving_s,
        distance_m,
        elevation_gain_m,
        avg_hr: None,
        max_hr: None,
        avg_speed_mps: None,
        avg_power_w: None,
        avg_cadence: None,
        calories: None,
    }
}

/// Inserisce un'attività; `n` rende unici percorso e SHA del file.
async fn seed(pool: &PgPool, user_id: i64, a: &ParsedActivity, n: u32) {
    activity_repository::insert(
        pool,
        user_id,
        a,
        FileFormat::Gpx,
        &format!("{user_id}/{n}.gpx"),
        &format!("sha-{n}"),
    )
    .await
    .unwrap();
}

/// Attività senza metriche opzionali alla data indicata.
async fn seed_simple(pool: &PgPool, user_id: i64, sport: Sport, start: &str, n: u32) {
    seed(pool, user_id, &parsed(sport, start, None, None, None), n).await;
}

async fn athlete(app: &TestApp) -> (i64, String) {
    app.register("athlete@example.com", "athlete", "athlete")
        .await
}

async fn get(app: &TestApp, token: &str, uri: &str) -> (StatusCode, Value) {
    app.request(Method::GET, uri, Some(token), None).await
}

fn assert_validation_failed(status: StatusCode, body: &Value) {
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "VALIDATION_FAILED", "{body}");
}

#[sqlx::test]
async fn summary_totals_and_by_sport(pool: PgPool) {
    let app = common::test_app(pool);
    let (id, token) = athlete(&app).await;

    // Corsa con tutte le metriche, corsa senza moving_s/distanza/dislivello,
    // bici con più tempo in movimento
    seed(
        &app.pool,
        id,
        &parsed(
            Sport::Running,
            "2026-11-03T08:00:00Z",
            Some(900),
            Some(5000.5),
            Some(40.0),
        ),
        1,
    )
    .await;
    seed(
        &app.pool,
        id,
        &parsed(Sport::Running, "2026-11-04T08:00:00Z", None, None, None),
        2,
    )
    .await;
    seed(
        &app.pool,
        id,
        &parsed(
            Sport::Cycling,
            "2026-11-05T08:00:00Z",
            Some(3000),
            Some(40000.0),
            Some(300.25),
        ),
        3,
    )
    .await;
    // Fuori intervallo: non conta
    seed(
        &app.pool,
        id,
        &parsed(
            Sport::Running,
            "2026-12-01T08:00:00Z",
            Some(100),
            Some(1.0),
            Some(1.0),
        ),
        4,
    )
    .await;

    let (status, body) = get(
        &app,
        &token,
        "/api/stats/summary?from=2026-11-01&to=2026-11-30",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert_eq!(body["from"], "2026-11-01");
    assert_eq!(body["to"], "2026-11-30");
    assert_eq!(body["granularity"], "day");
    assert_eq!(body["timezone"], "Europe/Rome");

    // moving_s nullo → elapsed_s (1000); i campi null valgono 0
    let totals = &body["totals"];
    assert_eq!(totals["count"], 3);
    assert_eq!(totals["distance_m"].as_f64().unwrap(), 45000.5);
    assert_eq!(totals["moving_s"], 900 + 1000 + 3000);
    assert_eq!(totals["elapsed_s"], 3000);
    assert_eq!(totals["elevation_gain_m"].as_f64().unwrap(), 340.25);

    // Ordinati per moving_s decrescente
    let by_sport = body["by_sport"].as_array().unwrap();
    assert_eq!(by_sport.len(), 2);
    assert_eq!(by_sport[0]["sport"], "cycling");
    assert_eq!(by_sport[0]["count"], 1);
    assert_eq!(by_sport[0]["distance_m"].as_f64().unwrap(), 40000.0);
    assert_eq!(by_sport[0]["moving_s"], 3000);
    assert_eq!(by_sport[1]["sport"], "running");
    assert_eq!(by_sport[1]["count"], 2);
    assert_eq!(by_sport[1]["distance_m"].as_f64().unwrap(), 5000.5);
    assert_eq!(by_sport[1]["moving_s"], 1900);

    // La serie giornaliera somma agli stessi totali
    let series = body["series"].as_array().unwrap();
    assert_eq!(series.len(), 30);
    assert_eq!(
        series
            .iter()
            .map(|p| p["count"].as_i64().unwrap())
            .sum::<i64>(),
        3
    );
    assert_eq!(series[2]["period_start"], "2026-11-03");
    assert_eq!(series[2]["moving_s"], 900);
}

#[sqlx::test]
async fn empty_range_returns_zeros(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = athlete(&app).await;

    let (status, body) = get(
        &app,
        &token,
        "/api/stats/summary?from=2026-11-01&to=2026-11-03",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["totals"]["count"], 0);
    assert_eq!(body["totals"]["distance_m"].as_f64().unwrap(), 0.0);
    assert_eq!(body["totals"]["moving_s"], 0);
    assert!(body["by_sport"].as_array().unwrap().is_empty());
    assert_eq!(body["series"].as_array().unwrap().len(), 3);
}

#[sqlx::test]
async fn series_zero_fills_empty_days(pool: PgPool) {
    let app = common::test_app(pool);
    let (id, token) = athlete(&app).await;
    seed(
        &app.pool,
        id,
        &parsed(
            Sport::Running,
            "2026-11-02T08:00:00Z",
            Some(600),
            Some(1000.0),
            Some(10.0),
        ),
        1,
    )
    .await;
    seed(
        &app.pool,
        id,
        &parsed(
            Sport::Running,
            "2026-11-07T08:00:00Z",
            Some(300),
            Some(500.0),
            None,
        ),
        2,
    )
    .await;

    let (status, body) = get(
        &app,
        &token,
        "/api/stats/summary?from=2026-11-01&to=2026-11-10",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let series = body["series"].as_array().unwrap();
    assert_eq!(series.len(), 10);
    let days: Vec<&str> = series
        .iter()
        .map(|p| p["period_start"].as_str().unwrap())
        .collect();
    assert_eq!(days[0], "2026-11-01");
    assert_eq!(days[9], "2026-11-10");

    let empty = series.iter().filter(|p| p["count"] == 0).count();
    assert_eq!(empty, 8);
    for p in series.iter().filter(|p| p["count"] == 0) {
        assert_eq!(p["distance_m"].as_f64().unwrap(), 0.0);
        assert_eq!(p["moving_s"], 0);
        assert_eq!(p["elevation_gain_m"].as_f64().unwrap(), 0.0);
    }
    assert_eq!(series[1]["count"], 1);
    assert_eq!(series[1]["distance_m"].as_f64().unwrap(), 1000.0);
    assert_eq!(series[1]["elevation_gain_m"].as_f64().unwrap(), 10.0);
    assert_eq!(series[6]["count"], 1);
    assert_eq!(series[6]["moving_s"], 300);
}

#[sqlx::test]
async fn auto_granularity_day_week_month(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = athlete(&app).await;

    // 10 giorni → day
    let (_, body) = get(
        &app,
        &token,
        "/api/stats/summary?from=2026-11-01&to=2026-11-10",
    )
    .await;
    assert_eq!(body["granularity"], "day");
    assert_eq!(body["series"].as_array().unwrap().len(), 10);

    // 31 giorni (estremo incluso) → ancora day; 32 → week
    let (_, body) = get(
        &app,
        &token,
        "/api/stats/summary?from=2026-12-01&to=2026-12-31",
    )
    .await;
    assert_eq!(body["granularity"], "day");
    let (_, body) = get(
        &app,
        &token,
        "/api/stats/summary?from=2026-12-01&to=2027-01-01",
    )
    .await;
    assert_eq!(body["granularity"], "week");

    // 90 giorni → week, periodi che iniziano di lunedì (2026-11-02 è un lunedì)
    let (_, body) = get(
        &app,
        &token,
        "/api/stats/summary?from=2026-11-04&to=2027-02-01",
    )
    .await;
    assert_eq!(body["granularity"], "week");
    let series = body["series"].as_array().unwrap();
    assert_eq!(series[0]["period_start"], "2026-11-02");
    assert_eq!(series[1]["period_start"], "2026-11-09");
    assert_eq!(series.len(), 14);

    // 180 giorni → week; 181 → month
    let (_, body) = get(
        &app,
        &token,
        "/api/stats/summary?from=2026-01-01&to=2026-06-29",
    )
    .await;
    assert_eq!(body["granularity"], "week");
    let (_, body) = get(
        &app,
        &token,
        "/api/stats/summary?from=2026-01-01&to=2026-06-30",
    )
    .await;
    assert_eq!(body["granularity"], "month");

    // 365 giorni → month, periodi al primo del mese
    let (_, body) = get(
        &app,
        &token,
        "/api/stats/summary?from=2026-01-01&to=2026-12-31",
    )
    .await;
    assert_eq!(body["granularity"], "month");
    let series = body["series"].as_array().unwrap();
    assert_eq!(series.len(), 12);
    assert_eq!(series[0]["period_start"], "2026-01-01");
    assert_eq!(series[11]["period_start"], "2026-12-01");
}

#[sqlx::test]
async fn explicit_granularity_overrides_auto(pool: PgPool) {
    let app = common::test_app(pool);
    let (id, token) = athlete(&app).await;
    // Domenica 8 e lunedì 9 novembre: settimane ISO diverse
    seed_simple(&app.pool, id, Sport::Running, "2026-11-08T10:00:00Z", 1).await;
    seed_simple(&app.pool, id, Sport::Running, "2026-11-09T10:00:00Z", 2).await;
    seed_simple(&app.pool, id, Sport::Running, "2026-12-15T10:00:00Z", 3).await;

    let (status, body) = get(
        &app,
        &token,
        "/api/stats/summary?from=2026-11-01&to=2026-12-31&granularity=week",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let series = body["series"].as_array().unwrap();
    let week = |start: &str| series.iter().find(|p| p["period_start"] == start).unwrap();
    assert_eq!(week("2026-11-02")["count"], 1);
    assert_eq!(week("2026-11-09")["count"], 1);

    // Mensile su un intervallo breve: il primo periodo parte dal primo del mese
    let (_, body) = get(
        &app,
        &token,
        "/api/stats/summary?from=2026-11-20&to=2026-12-20&granularity=month",
    )
    .await;
    assert_eq!(body["granularity"], "month");
    let series = body["series"].as_array().unwrap();
    assert_eq!(series.len(), 2);
    assert_eq!(series[0]["period_start"], "2026-11-01");
    assert_eq!(
        series[0]["count"], 0,
        "il 9 novembre è fuori dall'intervallo"
    );
    assert_eq!(series[1]["period_start"], "2026-12-01");
    assert_eq!(series[1]["count"], 1);

    // Giornaliero su un intervallo lungo
    let (_, body) = get(
        &app,
        &token,
        "/api/stats/summary?from=2026-01-01&to=2026-12-31&granularity=day",
    )
    .await;
    assert_eq!(body["series"].as_array().unwrap().len(), 365);
}

#[sqlx::test]
async fn stats_bucket_uses_athlete_timezone(pool: PgPool) {
    let app = common::test_app(pool);
    let (id, token) = athlete(&app).await;
    // 23:30Z del 28 marzo = 00:30 del 29 a Roma (CET, UTC+1)
    seed_simple(&app.pool, id, Sport::Running, "2026-03-28T23:30:00Z", 1).await;

    let uri = "/api/stats/summary?from=2026-03-27&to=2026-03-30";
    let (status, body) = get(&app, &token, uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["timezone"], "Europe/Rome");
    let series = body["series"].as_array().unwrap();
    let count_on =
        |day: &str| series.iter().find(|p| p["period_start"] == day).unwrap()["count"].clone();
    assert_eq!(count_on("2026-03-29"), 1);
    assert_eq!(count_on("2026-03-28"), 0);

    // Con il fuso UTC la stessa attività cade il 28
    sqlx::query("UPDATE user_profiles SET timezone = 'UTC' WHERE user_id = $1")
        .bind(id)
        .execute(&app.pool)
        .await
        .unwrap();
    let (_, body) = get(&app, &token, uri).await;
    assert_eq!(body["timezone"], "UTC");
    let series = body["series"].as_array().unwrap();
    let count_on =
        |day: &str| series.iter().find(|p| p["period_start"] == day).unwrap()["count"].clone();
    assert_eq!(count_on("2026-03-28"), 1);
    assert_eq!(count_on("2026-03-29"), 0);

    // Anche il filtro from/to è nel fuso dell'atleta: il 29 a Roma include l'attività
    sqlx::query("UPDATE user_profiles SET timezone = 'Europe/Rome' WHERE user_id = $1")
        .bind(id)
        .execute(&app.pool)
        .await
        .unwrap();
    let (_, body) = get(
        &app,
        &token,
        "/api/stats/summary?from=2026-03-29&to=2026-03-29",
    )
    .await;
    assert_eq!(body["totals"]["count"], 1);
    let (_, body) = get(
        &app,
        &token,
        "/api/stats/summary?from=2026-03-28&to=2026-03-28",
    )
    .await;
    assert_eq!(body["totals"]["count"], 0);
}

#[sqlx::test]
async fn sport_filter(pool: PgPool) {
    let app = common::test_app(pool);
    let (id, token) = athlete(&app).await;
    seed(
        &app.pool,
        id,
        &parsed(
            Sport::Running,
            "2026-11-03T08:00:00Z",
            Some(600),
            Some(1000.0),
            None,
        ),
        1,
    )
    .await;
    seed(
        &app.pool,
        id,
        &parsed(
            Sport::Cycling,
            "2026-11-04T08:00:00Z",
            Some(1200),
            Some(9000.0),
            None,
        ),
        2,
    )
    .await;

    let (status, body) = get(
        &app,
        &token,
        "/api/stats/summary?from=2026-11-01&to=2026-11-30&sport=cycling",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["totals"]["count"], 1);
    assert_eq!(body["totals"]["distance_m"].as_f64().unwrap(), 9000.0);
    let by_sport = body["by_sport"].as_array().unwrap();
    assert_eq!(by_sport.len(), 1);
    assert_eq!(by_sport[0]["sport"], "cycling");
    let series_total: i64 = body["series"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["count"].as_i64().unwrap())
        .sum();
    assert_eq!(series_total, 1);

    // Sport non valido → 400
    let (status, body) = get(
        &app,
        &token,
        "/api/stats/summary?from=2026-11-01&to=2026-11-30&sport=yoga",
    )
    .await;
    assert_validation_failed(status, &body);
}

#[sqlx::test]
async fn missing_from_is_400(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = athlete(&app).await;

    for uri in [
        "/api/stats/summary?to=2026-11-30",
        "/api/stats/summary?from=2026-11-01",
        "/api/stats/summary",
    ] {
        let (status, body) = get(&app, &token, uri).await;
        assert_validation_failed(status, &body);
    }

    // Data malformata
    let (status, body) = get(&app, &token, "/api/stats/summary?from=oggi&to=2026-11-30").await;
    assert_validation_failed(status, &body);
}

#[sqlx::test]
async fn from_after_to_is_400(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = athlete(&app).await;

    let (status, body) = get(
        &app,
        &token,
        "/api/stats/summary?from=2026-11-30&to=2026-11-01",
    )
    .await;
    assert_validation_failed(status, &body);
}

#[sqlx::test]
async fn range_longer_than_3660_days_is_400(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = athlete(&app).await;

    // 3660 giorni inclusi (2016-01-01 .. 2026-01-07) sono accettati, 3661 no
    let (status, body) = get(
        &app,
        &token,
        "/api/stats/summary?from=2016-01-01&to=2026-01-07&granularity=month",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = get(
        &app,
        &token,
        "/api/stats/summary?from=2016-01-01&to=2026-01-08&granularity=month",
    )
    .await;
    assert_validation_failed(status, &body);
}

#[sqlx::test]
async fn invalid_granularity_is_400(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = athlete(&app).await;

    for g in ["year", "Day", ""] {
        let uri = format!("/api/stats/summary?from=2026-11-01&to=2026-11-10&granularity={g}");
        let (status, body) = get(&app, &token, &uri).await;
        assert_validation_failed(status, &body);
    }
}

#[sqlx::test]
async fn stats_require_auth(pool: PgPool) {
    let app = common::test_app(pool);
    let (status, _) = app
        .request(
            Method::GET,
            "/api/stats/summary?from=2026-11-01&to=2026-11-10",
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn coach_active_link_ok_else_403(pool: PgPool) {
    let app = common::test_app(pool);
    let (athlete_id, athlete_token) = athlete(&app).await;
    seed(
        &app.pool,
        athlete_id,
        &parsed(
            Sport::Running,
            "2026-03-28T23:30:00Z",
            Some(600),
            Some(2000.0),
            None,
        ),
        1,
    )
    .await;
    let (_, coach_token) = app
        .register("coach@example.com", "allenatore", "coach")
        .await;
    let uri = format!("/api/stats/summary?from=2026-03-27&to=2026-03-30&athlete_id={athlete_id}");

    // Senza link → 403
    let (status, body) = get(&app, &coach_token, &uri).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "ATHLETE_ACCESS_DENIED");

    // Richiesta pending → ancora 403
    let (status, link) = app
        .request(
            Method::POST,
            "/api/coach-links",
            Some(&athlete_token),
            Some(json!({ "coach": "allenatore" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{link}");
    let (status, body) = get(&app, &coach_token, &uri).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // Link attivo → 200; il fuso è quello dell'atleta (Roma), non del coach
    sqlx::query("UPDATE user_profiles SET timezone = 'America/Los_Angeles' WHERE user_id <> $1")
        .bind(athlete_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let (status, body) = app
        .request(
            Method::POST,
            &format!("/api/coach-links/{}/accept", link["id"]),
            Some(&coach_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = get(&app, &coach_token, &uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["timezone"], "Europe/Rome");
    assert_eq!(body["totals"]["count"], 1);
    let series = body["series"].as_array().unwrap();
    let day = series
        .iter()
        .find(|p| p["period_start"] == "2026-03-29")
        .unwrap();
    assert_eq!(day["count"], 1);

    // Senza athlete_id il coach vede le proprie statistiche (vuote)
    let (_, body) = get(
        &app,
        &coach_token,
        "/api/stats/summary?from=2026-03-27&to=2026-03-30",
    )
    .await;
    assert_eq!(body["totals"]["count"], 0);

    // Un altro atleta non può leggere le statistiche altrui
    let (_, other) = app.register("b@example.com", "atleta-b", "athlete").await;
    let (status, body) = get(&app, &other, &uri).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}
