mod common;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use common::TestApp;

const RUN_GPX: &[u8] = include_bytes!("fixtures/run.gpx");
const RIDE_TCX: &[u8] = include_bytes!("fixtures/ride.tcx");

/// Registra un atleta e ne ritorna `(id, token)`.
async fn athlete(app: &TestApp) -> (i64, String) {
    app.register("athlete@example.com", "athlete", "athlete")
        .await
}

/// Carica i file, attende la fine dell'import e ritorna gli id delle attività create.
async fn import(app: &TestApp, token: &str, files: &[(&str, Vec<u8>)]) -> Vec<i64> {
    let (status, body) = app.upload(token, files).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let result = app.wait_import(token, body["id"].as_i64().unwrap()).await;
    result["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["activity_id"].as_i64().expect("attività non importata"))
        .collect()
}

/// Inserisce direttamente un'attività (l'`n` rende unico lo SHA del file).
async fn insert(pool: &PgPool, user_id: i64, sport: &str, start_time: &str, n: u32) {
    sqlx::query(
        "INSERT INTO activities (user_id, sport, sport_raw, start_time, elapsed_s,
                                 file_format, file_path, file_sha256)
         VALUES ($1, $2, $2, $3::timestamptz, 600, 'gpx', $4, $5)",
    )
    .bind(user_id)
    .bind(sport)
    .bind(start_time)
    .bind(format!("{user_id}/{n}.gpx"))
    .bind(format!("sha-{n}"))
    .execute(pool)
    .await
    .unwrap();
}

async fn get(app: &TestApp, token: &str, uri: &str) -> (StatusCode, Value) {
    app.request(Method::GET, uri, Some(token), None).await
}

fn assert_validation_failed(status: StatusCode, body: &Value) {
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "VALIDATION_FAILED", "{body}");
}

/// Crea un coach e lo collega (link `active`) all'atleta; ritorna `(id, token)` del coach.
async fn linked_coach(app: &TestApp, athlete_token: &str) -> (i64, String) {
    let (coach_id, coach_token) = app
        .register("coach@example.com", "allenatore", "coach")
        .await;
    let (status, body) = app
        .request(
            Method::POST,
            "/api/coach-links",
            Some(athlete_token),
            Some(json!({ "coach": "allenatore" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let (status, body) = app
        .request(
            Method::POST,
            &format!("/api/coach-links/{}/accept", body["id"]),
            Some(&coach_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    (coach_id, coach_token)
}

#[sqlx::test]
async fn list_own_activities_desc(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = athlete(&app).await;

    // run.gpx inizia alle 07:00Z, ride.tcx alle 08:00Z
    let imported = import(
        &app,
        &token,
        &[
            ("run.gpx", RUN_GPX.to_vec()),
            ("ride.tcx", RIDE_TCX.to_vec()),
        ],
    )
    .await;
    assert_eq!(imported.len(), 2);

    let (status, body) = get(&app, &token, "/api/activities").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 2);
    assert_eq!(body["page"], 1);
    assert_eq!(body["per_page"], 20);

    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["sport"], "cycling");
    assert_eq!(items[0]["start_time"], "2026-01-10T08:00:00Z");
    assert_eq!(items[0]["file_format"], "tcx");
    assert_eq!(items[0]["source"], "file_upload");
    assert_eq!(items[1]["sport"], "running");
    assert_eq!(items[1]["start_time"], "2026-01-10T07:00:00Z");
    assert!(items[1]["elapsed_s"].as_i64().unwrap() > 0);
    // Il percorso del file originale e l'hash non sono esposti
    assert!(items[0].get("file_path").is_none() && items[0].get("file_sha256").is_none());

    // Un altro atleta non vede queste attività
    let (_, other) = app.register("b@example.com", "atleta-b", "athlete").await;
    let (_, body) = get(&app, &other, "/api/activities").await;
    assert_eq!(body["total"], 0);
    assert!(body["items"].as_array().unwrap().is_empty());
}

#[sqlx::test]
async fn list_filters_by_date_and_sport(pool: PgPool) {
    let app = common::test_app(pool);
    let (id, token) = athlete(&app).await;

    // Fuso di default Europe/Rome (UTC+1 in inverno)
    insert(&app.pool, id, "running", "2026-01-09T22:59:59Z", 1).await; // 9 gen 23:59:59 Roma
    insert(&app.pool, id, "running", "2026-01-09T23:00:00Z", 2).await; // 10 gen 00:00:00 Roma
    insert(&app.pool, id, "cycling", "2026-01-10T22:59:59Z", 3).await; // 10 gen 23:59:59 Roma
    insert(&app.pool, id, "running", "2026-01-10T23:00:00Z", 4).await; // 11 gen 00:00:00 Roma

    let count = |body: &Value| body["total"].as_i64().unwrap();

    // Estremi inclusivi, nel fuso dell'atleta
    let (status, body) = get(
        &app,
        &token,
        "/api/activities?from=2026-01-10&to=2026-01-10",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(count(&body), 2);
    let starts: Vec<&str> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["start_time"].as_str().unwrap())
        .collect();
    assert_eq!(starts, ["2026-01-10T22:59:59Z", "2026-01-09T23:00:00Z"]);

    // Solo `from` / solo `to`
    let (_, body) = get(&app, &token, "/api/activities?from=2026-01-11").await;
    assert_eq!(count(&body), 1);
    let (_, body) = get(&app, &token, "/api/activities?to=2026-01-09").await;
    assert_eq!(count(&body), 1);

    // Sport (il totale rispetta il filtro)
    let (_, body) = get(&app, &token, "/api/activities?sport=running").await;
    assert_eq!(count(&body), 3);
    let (_, body) = get(
        &app,
        &token,
        "/api/activities?sport=cycling&from=2026-01-10&to=2026-01-10",
    )
    .await;
    assert_eq!(count(&body), 1);
    assert_eq!(body["items"][0]["sport"], "cycling");
    let (_, body) = get(&app, &token, "/api/activities?sport=swimming").await;
    assert_eq!(count(&body), 0);

    // Cambiando fuso cambia il giorno di appartenenza: a Los Angeles (UTC-8)
    // la prima attività è del 9 gen 14:59, l'ultima del 10 gen 15:00
    sqlx::query("UPDATE user_profiles SET timezone = 'America/Los_Angeles' WHERE user_id = $1")
        .bind(id)
        .execute(&app.pool)
        .await
        .unwrap();
    let (_, body) = get(
        &app,
        &token,
        "/api/activities?from=2026-01-09&to=2026-01-09",
    )
    .await;
    assert_eq!(count(&body), 2);
    let (_, body) = get(
        &app,
        &token,
        "/api/activities?from=2026-01-10&to=2026-01-10",
    )
    .await;
    assert_eq!(count(&body), 2);

    // Parametri non validi → 400 VALIDATION_FAILED
    for bad in [
        "from=2026-01-11&to=2026-01-10",
        "from=10/01/2026",
        "to=ieri",
        "sport=Running",
        "sport=curling",
        "page=0",
        "page=-1",
        "page=abc",
        "per_page=abc",
    ] {
        let (status, body) = get(&app, &token, &format!("/api/activities?{bad}")).await;
        assert_validation_failed(status, &body);
    }
}

/// Date con anno fuori da 1900..=2200 → 400; gli estremi sono accettati.
#[sqlx::test]
async fn list_date_year_out_of_range_is_400(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = athlete(&app).await;

    for bad in [
        "from=1899-12-31",
        "to=2201-01-01",
        "from=0001-01-01&to=2026-01-01",
        "from=2026-01-01&to=9999-12-31",
    ] {
        let (status, body) = get(&app, &token, &format!("/api/activities?{bad}")).await;
        assert_validation_failed(status, &body);
    }
    let (status, body) = get(
        &app,
        &token,
        "/api/activities?from=1900-01-01&to=2200-12-31",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[sqlx::test]
async fn list_paginates_with_total(pool: PgPool) {
    let app = common::test_app(pool);
    let (id, token) = athlete(&app).await;
    for n in 1..=5 {
        insert(
            &app.pool,
            id,
            "running",
            &format!("2026-01-0{n}T10:00:00Z"),
            n,
        )
        .await;
    }

    // Dal più recente: giorno 5, 4 | 3, 2 | 1
    let (_, p1) = get(&app, &token, "/api/activities?per_page=2").await;
    let (_, p2) = get(&app, &token, "/api/activities?per_page=2&page=2").await;
    let (_, p3) = get(&app, &token, "/api/activities?per_page=2&page=3").await;
    assert_eq!(
        (
            p1["total"].as_i64(),
            p1["page"].as_i64(),
            p1["per_page"].as_i64()
        ),
        (Some(5), Some(1), Some(2))
    );
    assert_eq!(p2["page"], 2);
    let days = |p: &Value| -> Vec<String> {
        p["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["start_time"].as_str().unwrap()[..10].to_string())
            .collect()
    };
    assert_eq!(days(&p1), ["2026-01-05", "2026-01-04"]);
    assert_eq!(days(&p2), ["2026-01-03", "2026-01-02"]);
    assert_eq!(days(&p3), ["2026-01-01"]);
    assert_eq!(p3["total"], 5);

    // Oltre l'ultima pagina: elenco vuoto, totale invariato
    let (status, p9) = get(&app, &token, "/api/activities?per_page=2&page=9").await;
    assert_eq!(status, StatusCode::OK);
    assert!(p9["items"].as_array().unwrap().is_empty());
    assert_eq!(p9["total"], 5);

    // per_page = 100 è ammesso
    let (status, body) = get(&app, &token, "/api/activities?per_page=100").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 5);
}

#[sqlx::test]
async fn coach_with_active_link_can_list_athlete(pool: PgPool) {
    let app = common::test_app(pool);
    let (athlete_id, athlete_token) = athlete(&app).await;
    let (_, coach_token) = linked_coach(&app, &athlete_token).await;
    // Il giorno si calcola nel fuso dell'atleta (Roma), non in quello del coach
    sqlx::query("UPDATE user_profiles SET timezone = 'America/Los_Angeles' WHERE user_id <> $1")
        .bind(athlete_id)
        .execute(&app.pool)
        .await
        .unwrap();
    insert(&app.pool, athlete_id, "running", "2026-01-09T23:30:00Z", 1).await; // 10 gen Roma

    let uri = format!("/api/activities?athlete_id={athlete_id}&from=2026-01-10&to=2026-01-10");
    let (status, body) = get(&app, &coach_token, &uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["sport"], "running");

    // Senza athlete_id il coach vede le proprie attività (nessuna)
    let (_, body) = get(&app, &coach_token, "/api/activities").await;
    assert_eq!(body["total"], 0);
}

#[sqlx::test]
async fn coach_without_link_gets_403_athlete_access_denied(pool: PgPool) {
    let app = common::test_app(pool);
    let (athlete_id, athlete_token) = athlete(&app).await;
    insert(&app.pool, athlete_id, "running", "2026-01-10T07:00:00Z", 1).await;
    let (_, coach_token) = app
        .register("coach@example.com", "allenatore", "coach")
        .await;

    let uri = format!("/api/activities?athlete_id={athlete_id}");
    let (status, body) = get(&app, &coach_token, &uri).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "ATHLETE_ACCESS_DENIED");

    // Richiesta pending: ancora nessun accesso
    let (status, _) = app
        .request(
            Method::POST,
            "/api/coach-links",
            Some(&athlete_token),
            Some(json!({ "coach": "allenatore" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, body) = get(&app, &coach_token, &uri).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "ATHLETE_ACCESS_DENIED");
}

#[sqlx::test]
async fn athlete_cannot_list_other_athlete(pool: PgPool) {
    let app = common::test_app(pool);
    let (a_id, _) = athlete(&app).await;
    let (_, b_token) = app.register("b@example.com", "atleta-b", "athlete").await;
    insert(&app.pool, a_id, "running", "2026-01-10T07:00:00Z", 1).await;

    let (status, body) = get(
        &app,
        &b_token,
        &format!("/api/activities?athlete_id={a_id}"),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "ATHLETE_ACCESS_DENIED");
}

#[sqlx::test]
async fn delete_own_activity_removes_file(pool: PgPool) {
    let app = common::test_app(pool);
    let (user_id, token) = athlete(&app).await;
    let imported = import(&app, &token, &[("run.gpx", RUN_GPX.to_vec())]).await;
    let activity_id = imported[0];

    let file_path: String = sqlx::query_scalar("SELECT file_path FROM activities WHERE id = $1")
        .bind(activity_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(file_path.starts_with(&format!("{user_id}/")));
    let on_disk = app.upload_dir.path().join(&file_path);
    assert!(on_disk.exists());

    let (status, body) = app
        .request(
            Method::DELETE,
            &format!("/api/activities/{activity_id}"),
            Some(&token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert!(!on_disk.exists(), "il file originale doveva essere rimosso");

    let (_, body) = get(&app, &token, "/api/activities").await;
    assert_eq!(body["total"], 0);

    // Già eliminata → 404
    let (status, body) = app
        .request(
            Method::DELETE,
            &format!("/api/activities/{activity_id}"),
            Some(&token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "NOT_FOUND");

    // Lo stesso file può essere reimportato (la riga non esiste più)
    let again = import(&app, &token, &[("run.gpx", RUN_GPX.to_vec())]).await;
    assert_eq!(again.len(), 1);
    assert!(on_disk.exists());
}

#[sqlx::test]
async fn cannot_delete_others_activity_404(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = athlete(&app).await;
    let (_, coach_token) = linked_coach(&app, &token).await;
    let (_, other_token) = app.register("b@example.com", "atleta-b", "athlete").await;
    let activity_id = import(&app, &token, &[("run.gpx", RUN_GPX.to_vec())]).await[0];
    let uri = format!("/api/activities/{activity_id}");

    // Né un altro atleta né il coach collegato possono eliminarla
    for intruder in [&other_token, &coach_token] {
        let (status, body) = app
            .request(Method::DELETE, &uri, Some(intruder), None)
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert_eq!(body["code"], "NOT_FOUND");
    }

    // Id malformato → 400 JSON
    let (status, body) = app
        .request(Method::DELETE, "/api/activities/abc", Some(&token), None)
        .await;
    assert_validation_failed(status, &body);

    // Senza token → 401
    let (status, _) = app.request(Method::DELETE, &uri, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // L'attività e il file sono intatti
    let (_, body) = get(&app, &token, "/api/activities").await;
    assert_eq!(body["total"], 1);
    let file_path: String = sqlx::query_scalar("SELECT file_path FROM activities WHERE id = $1")
        .bind(activity_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(app.upload_dir.path().join(file_path).exists());
}

#[sqlx::test]
async fn per_page_over_100_is_400(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = athlete(&app).await;

    for per_page in ["101", "0", "-5"] {
        let (status, body) = get(
            &app,
            &token,
            &format!("/api/activities?per_page={per_page}"),
        )
        .await;
        assert_validation_failed(status, &body);
    }
}
