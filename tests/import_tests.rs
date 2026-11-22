mod common;

use std::io::{Cursor, Write};

use axum::http::{Method, StatusCode};
use rust_postgre::errors::AppError;
use rust_postgre::parsing::{FileFormat, parse};
use rust_postgre::repositories::{activity_repository, import_repository};
use serde_json::Value;
use sqlx::PgPool;

use common::TestApp;

const RUN_GPX: &[u8] = include_bytes!("fixtures/run.gpx");
const RIDE_TCX: &[u8] = include_bytes!("fixtures/ride.tcx");
const RIDE_SAME_AS_TCX_GPX: &[u8] = include_bytes!("fixtures/ride_same_as_tcx.gpx");
const MALFORMED_FIT: &[u8] = include_bytes!("fixtures/malformed.fit");

/// Registra un atleta e ne ritorna `(id, token)`.
async fn athlete(app: &TestApp) -> (i64, String) {
    app.register("athlete@example.com", "athlete", "athlete")
        .await
}

/// Carica i file, verifica il 202 e attende la fine dell'import.
async fn import(app: &TestApp, token: &str, files: &[(&str, Vec<u8>)]) -> Value {
    let (status, body) = app.upload(token, files).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["status"], "processing");
    let id = body["id"].as_i64().expect("id import mancante");
    app.wait_import(token, id).await
}

/// Item dell'import con il nome indicato.
fn item<'a>(import: &'a Value, filename: &str) -> &'a Value {
    import["items"]
        .as_array()
        .expect("items mancanti")
        .iter()
        .find(|i| i["filename"] == filename)
        .unwrap_or_else(|| panic!("item {filename} mancante in {import}"))
}

/// Nessun file temporaneo di upload deve restare su disco.
fn assert_no_temp_files(app: &TestApp) {
    let tmp = app.upload_dir.path().join("tmp");
    let left: Vec<_> = std::fs::read_dir(&tmp)
        .map(|entries| entries.map(|e| e.unwrap().path()).collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "file temporanei rimasti: {left:?}");
}

#[sqlx::test]
async fn upload_gpx_imports_activity(pool: PgPool) {
    let app = common::test_app(pool);
    let (user_id, token) = athlete(&app).await;

    let result = import(&app, &token, &[("run.gpx", RUN_GPX.to_vec())]).await;
    assert_eq!(result["items"].as_array().unwrap().len(), 1, "{result}");
    let run = item(&result, "run.gpx");
    assert_eq!(run["status"], "imported", "{result}");
    assert!(run["error_code"].is_null());
    let activity_id = run["activity_id"].as_i64().expect("activity_id mancante");
    assert!(result["finished_at"].is_string());

    // Il file originale è salvato sotto upload_dir/{user_id}/
    let file_path: String =
        sqlx::query_scalar("SELECT file_path FROM activities WHERE id = $1 AND user_id = $2")
            .bind(activity_id)
            .bind(user_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(file_path.starts_with(&format!("{user_id}/")), "{file_path}");
    let stored = app.upload_dir.path().join(&file_path);
    assert_eq!(std::fs::read(stored).unwrap(), RUN_GPX);
    assert_no_temp_files(&app);
}

#[sqlx::test]
async fn upload_same_file_twice_second_is_duplicate(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = athlete(&app).await;

    let first = import(&app, &token, &[("run.gpx", RUN_GPX.to_vec())]).await;
    let first_id = item(&first, "run.gpx")["activity_id"].as_i64().unwrap();

    let second = import(&app, &token, &[("run.gpx", RUN_GPX.to_vec())]).await;
    let dup = item(&second, "run.gpx");
    assert_eq!(dup["status"], "duplicate", "{second}");
    assert_eq!(dup["activity_id"].as_i64(), Some(first_id));
}

/// Review Focus 2: la stessa attività da due formati diversi (inizio a +30 s) è un duplicato.
#[sqlx::test]
async fn same_activity_fit_and_tcx_is_duplicate(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = athlete(&app).await;

    let first = import(&app, &token, &[("ride.tcx", RIDE_TCX.to_vec())]).await;
    let ride = item(&first, "ride.tcx");
    assert_eq!(ride["status"], "imported", "{first}");
    let ride_id = ride["activity_id"].as_i64().unwrap();

    let second = import(
        &app,
        &token,
        &[("ride_same_as_tcx.gpx", RIDE_SAME_AS_TCX_GPX.to_vec())],
    )
    .await;
    let dup = item(&second, "ride_same_as_tcx.gpx");
    assert_eq!(dup["status"], "duplicate", "{second}");
    assert_eq!(dup["activity_id"].as_i64(), Some(ride_id));

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM activities")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test]
async fn malformed_file_fails_others_succeed(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = athlete(&app).await;

    let result = import(
        &app,
        &token,
        &[
            ("malformed.fit", MALFORMED_FIT.to_vec()),
            ("run.gpx", RUN_GPX.to_vec()),
        ],
    )
    .await;
    assert_eq!(result["status"], "completed");
    let bad = item(&result, "malformed.fit");
    assert_eq!(bad["status"], "failed", "{result}");
    assert_eq!(bad["error_code"], "PARSE_ERROR");
    assert!(bad["activity_id"].is_null());
    assert_eq!(item(&result, "run.gpx")["status"], "imported", "{result}");
}

#[sqlx::test]
async fn unsupported_extension_fails_unsupported_format(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = athlete(&app).await;

    let result = import(&app, &token, &[("notes.txt", b"hello".to_vec())]).await;
    let notes = item(&result, "notes.txt");
    assert_eq!(notes["status"], "failed", "{result}");
    assert_eq!(notes["error_code"], "UNSUPPORTED_FORMAT");
    assert_no_temp_files(&app);
}

#[sqlx::test]
async fn zip_upload_expands(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = athlete(&app).await;

    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in [("run.gpx", RUN_GPX), ("sub/ride.tcx", RIDE_TCX)] {
        writer.start_file(name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    let zip_bytes = writer.finish().unwrap().into_inner();

    let result = import(&app, &token, &[("export.zip", zip_bytes)]).await;
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{result}");
    assert_eq!(item(&result, "run.gpx")["status"], "imported", "{result}");
    assert_eq!(item(&result, "ride.tcx")["status"], "imported", "{result}");
    assert_no_temp_files(&app);
}

#[sqlx::test]
async fn coach_cannot_upload(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = app.register("coach@example.com", "coach", "coach").await;

    let (status, body) = app.upload(&token, &[("run.gpx", RUN_GPX.to_vec())]).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "ROLE_FORBIDDEN");
}

#[sqlx::test]
async fn no_files_is_400_no_files(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = athlete(&app).await;

    let (status, body) = app.upload(&token, &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "NO_FILES");

    // Un campo con nome diverso da `files` non conta come file
    let (status, body) = app
        .upload_fields(&token, &[("other", "run.gpx", RUN_GPX)])
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "NO_FILES");
    assert_no_temp_files(&app);
}

#[sqlx::test]
async fn other_user_cannot_read_import(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = athlete(&app).await;
    let (_, other) = app.register("other@example.com", "other", "athlete").await;

    let (status, body) = app.upload(&token, &[("run.gpx", RUN_GPX.to_vec())]).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let id = body["id"].as_i64().unwrap();
    app.wait_import(&token, id).await;

    let (status, body) = app
        .request(
            Method::GET,
            &format!("/api/imports/{id}"),
            Some(&other),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "NOT_FOUND");

    // Nemmeno nell'elenco dell'altro utente
    let (status, body) = app
        .request(Method::GET, "/api/imports", Some(&other), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 0);
}

#[sqlx::test]
async fn list_imports_returns_latest_without_items(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = athlete(&app).await;

    let first = import(&app, &token, &[("run.gpx", RUN_GPX.to_vec())]).await;
    let second = import(&app, &token, &[("ride.tcx", RIDE_TCX.to_vec())]).await;

    let (status, body) = app
        .request(Method::GET, "/api/imports", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let list = body.as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["id"], second["id"]);
    assert_eq!(list[1]["id"], first["id"]);
    assert_eq!(list[0]["status"], "completed");
    assert!(list[0].get("items").is_none(), "{body}");
}

/// Review Focus 3: un upload oltre il limite di default di axum (2 MiB) viene accettato.
#[sqlx::test]
async fn upload_larger_than_axum_default_limit(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = athlete(&app).await;

    // run.gpx con ~3 MB di commento XML dopo la dichiarazione
    let text = std::str::from_utf8(RUN_GPX).unwrap();
    let (decl, rest) = text.split_once("?>").unwrap();
    let big = format!("{decl}?>\n<!-- {} -->{rest}", "x".repeat(3 * 1024 * 1024));
    assert!(big.len() > 3 * 1024 * 1024);

    let result = import(&app, &token, &[("big.gpx", big.into_bytes())]).await;
    assert_eq!(item(&result, "big.gpx")["status"], "imported", "{result}");
}

#[sqlx::test]
async fn upload_over_max_upload_bytes_is_413(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = athlete(&app).await;

    // test_app usa MAX_UPLOAD_BYTES = 10 MiB
    let (status, body) = app
        .upload(&token, &[("huge.gpx", vec![b' '; 11 * 1024 * 1024])])
        .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    assert_eq!(body["code"], "UPLOAD_TOO_LARGE");
    assert_eq!(body["status"], 413);
    assert_no_temp_files(&app);

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM imports")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[sqlx::test]
async fn fail_interrupted_marks_pending(pool: PgPool) {
    let app = common::test_app(pool.clone());
    let (user_id, _) = athlete(&app).await;

    let import_id: i64 = sqlx::query_scalar(
        "INSERT INTO imports (user_id, status) VALUES ($1, 'processing') RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO import_items (import_id, filename, status) VALUES ($1, 'a.gpx', 'pending')",
    )
    .bind(import_id)
    .execute(&pool)
    .await
    .unwrap();
    // Un import già concluso non viene toccato
    let done_id: i64 = sqlx::query_scalar(
        "INSERT INTO imports (user_id, status, finished_at) VALUES ($1, 'completed', NOW()) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let affected = import_repository::fail_interrupted(&pool).await.unwrap();
    assert_eq!(affected, 1);

    let (status, finished): (String, Option<chrono::DateTime<chrono::Utc>>) =
        sqlx::query_as("SELECT status, finished_at FROM imports WHERE id = $1")
            .bind(import_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "failed");
    assert!(finished.is_some());

    let (item_status, code): (String, Option<String>) =
        sqlx::query_as("SELECT status, error_code FROM import_items WHERE import_id = $1")
            .bind(import_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(item_status, "failed");
    assert_eq!(code.as_deref(), Some("IMPORT_INTERRUPTED"));

    let done_status: String = sqlx::query_scalar("SELECT status FROM imports WHERE id = $1")
        .bind(done_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(done_status, "completed");
}

/// Chiusura dell'import: gli item rimasti `pending` diventano `failed`/`INTERNAL`.
#[sqlx::test]
async fn complete_fails_leftover_pending_items(pool: PgPool) {
    let app = common::test_app(pool.clone());
    let (user_id, _) = athlete(&app).await;

    let import_id: i64 = sqlx::query_scalar(
        "INSERT INTO imports (user_id, status) VALUES ($1, 'processing') RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO import_items (import_id, filename, status) \
         VALUES ($1, 'a.gpx', 'pending'), ($1, 'b.gpx', 'imported')",
    )
    .bind(import_id)
    .execute(&pool)
    .await
    .unwrap();

    import_repository::complete(&pool, import_id).await.unwrap();

    let status: String = sqlx::query_scalar("SELECT status FROM imports WHERE id = $1")
        .bind(import_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "completed");

    let items: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT filename, status, error_code FROM import_items WHERE import_id = $1 ORDER BY filename",
    )
    .bind(import_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        items[0],
        ("a.gpx".into(), "failed".into(), Some("INTERNAL".into()))
    );
    assert_eq!(items[1], ("b.gpx".into(), "imported".into(), None));
}

/// Inserire due volte lo stesso file (stesso utente e SHA-256) segnala `DUPLICATE_FILE`.
#[sqlx::test]
async fn insert_same_hash_reports_duplicate_file(pool: PgPool) {
    let app = common::test_app(pool.clone());
    let (user_id, _) = athlete(&app).await;

    let activity = parse(FileFormat::Gpx, RUN_GPX).unwrap();
    let sha = "a".repeat(64);
    activity_repository::insert(&pool, user_id, &activity, FileFormat::Gpx, "x/a.gpx", &sha)
        .await
        .unwrap();

    let err =
        activity_repository::insert(&pool, user_id, &activity, FileFormat::Gpx, "x/a.gpx", &sha)
            .await
            .unwrap_err();
    match err {
        AppError::Coded { status, code, .. } => {
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(code, activity_repository::DUPLICATE_FILE);
        }
        other => panic!("atteso DUPLICATE_FILE, ottenuto {other:?}"),
    }
}

/// Un solo import attivo per utente: con un import `processing` un nuovo upload è 409.
#[sqlx::test]
async fn upload_while_import_processing_is_409(pool: PgPool) {
    let app = common::test_app(pool);
    let (user_id, token) = athlete(&app).await;
    sqlx::query("INSERT INTO imports (user_id, status) VALUES ($1, 'processing')")
        .bind(user_id)
        .execute(&app.pool)
        .await
        .unwrap();

    let (status, body) = app.upload(&token, &[("run.gpx", RUN_GPX.to_vec())]).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "IMPORT_IN_PROGRESS");
    assert_eq!(body["status"], 409);
    assert_no_temp_files(&app);

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM imports")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);

    // Un altro utente può importare nel frattempo
    let (_, other) = app.register("other@example.com", "other", "athlete").await;
    let result = import(&app, &other, &[("run.gpx", RUN_GPX.to_vec())]).await;
    assert_eq!(item(&result, "run.gpx")["status"], "imported", "{result}");
}

/// Il vincolo a DB (non solo il controllo preventivo) impedisce un secondo import in corso.
#[sqlx::test]
async fn create_second_processing_import_is_import_in_progress(pool: PgPool) {
    let app = common::test_app(pool.clone());
    let (user_id, _) = athlete(&app).await;

    let first = import_repository::create(&pool, user_id).await.unwrap();
    match import_repository::create(&pool, user_id).await.unwrap_err() {
        AppError::Coded { status, code, .. } => {
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(code, import_repository::IMPORT_IN_PROGRESS);
        }
        other => panic!("atteso IMPORT_IN_PROGRESS, ottenuto {other:?}"),
    }

    // Concluso il primo, se ne può creare un altro
    import_repository::complete(&pool, first.id).await.unwrap();
    import_repository::create(&pool, user_id).await.unwrap();
}

/// Due upload concorrenti dello stesso utente: uno è accettato, l'altro è 409.
///
/// Senza permessi di elaborazione il primo import resta `processing`: l'esito
/// non dipende dalla velocità dell'elaborazione.
#[sqlx::test]
async fn concurrent_uploads_of_same_user_one_is_rejected(pool: PgPool) {
    let app = common::test_app_with_import_permits(pool, 0);
    let (_, token) = athlete(&app).await;

    let tcx = vec![("ride.tcx", RIDE_TCX.to_vec())];
    let gpx = vec![("ride_same_as_tcx.gpx", RIDE_SAME_AS_TCX_GPX.to_vec())];
    let ((s1, b1), (s2, b2)) = tokio::join!(app.upload(&token, &tcx), app.upload(&token, &gpx));
    let mut statuses = [s1, s2];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::ACCEPTED, StatusCode::CONFLICT],
        "{b1} {b2}"
    );
    let rejected = if s1 == StatusCode::CONFLICT { &b1 } else { &b2 };
    assert_eq!(rejected["code"], "IMPORT_IN_PROGRESS");

    // In attesa di un permesso l'import accettato resta `processing`
    let accepted = if s1 == StatusCode::ACCEPTED { &b1 } else { &b2 };
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let (status, body) = app
        .request(
            Method::GET,
            &format!("/api/imports/{}", accepted["id"]),
            Some(&token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "processing", "{body}");
    assert_eq!(body["items"].as_array().unwrap().len(), 0, "{body}");
}
