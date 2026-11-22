mod common;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

/// Invia POST /api/auth/register con il body indicato.
async fn post_register(app: &common::TestApp, body: Value) -> (StatusCode, Value) {
    app.request(Method::POST, "/api/auth/register", None, Some(body))
        .await
}

fn valid_body() -> Value {
    json!({
        "email": "atleta@example.com",
        "username": "atleta",
        "password": "password123",
        "role": "athlete"
    })
}

#[sqlx::test]
async fn register_athlete_returns_token_and_role(pool: PgPool) {
    let app = common::test_app(pool);
    let (status, body) = post_register(&app, valid_body()).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(body["token"].as_str().is_some_and(|t| !t.is_empty()));
    assert_eq!(body["user"]["role"], "athlete");
    assert_eq!(body["user"]["email"], "atleta@example.com");
    assert!(body["user"].get("password_hash").is_none());
}

#[sqlx::test]
async fn register_coach(pool: PgPool) {
    let app = common::test_app(pool);
    let mut body = valid_body();
    body["role"] = json!("coach");
    let (status, body) = post_register(&app, body).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["user"]["role"], "coach");
}

#[sqlx::test]
async fn register_creates_profile_row_with_defaults(pool: PgPool) {
    let app = common::test_app(pool.clone());
    let (id, _) = app.register("a@example.com", "atleta", "athlete").await;
    let row = sqlx::query!(
        "SELECT language, timezone FROM user_profiles WHERE user_id = $1",
        id
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.language, "it");
    assert_eq!(row.timezone, "Europe/Rome");
}

#[sqlx::test]
async fn register_invalid_role_is_400(pool: PgPool) {
    let app = common::test_app(pool);
    let mut body = valid_body();
    body["role"] = json!("admin");
    let (status, body) = post_register(&app, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION_FAILED");
}

#[sqlx::test]
async fn register_missing_role_is_400(pool: PgPool) {
    let app = common::test_app(pool);
    let mut body = valid_body();
    body.as_object_mut().unwrap().remove("role");
    let (status, body) = post_register(&app, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION_FAILED");
}

#[sqlx::test]
async fn register_short_password_is_400(pool: PgPool) {
    let app = common::test_app(pool);
    let mut body = valid_body();
    body["password"] = json!("short");
    let (status, body) = post_register(&app, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION_FAILED");
    assert!(body["error"].as_str().unwrap().contains("password"));
}

#[sqlx::test]
async fn register_invalid_email_and_username_are_400(pool: PgPool) {
    let app = common::test_app(pool);

    let mut body = valid_body();
    body["email"] = json!("senza-chiocciola");
    let (status, body) = post_register(&app, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION_FAILED");
    assert!(body["error"].as_str().unwrap().contains("email"));

    for bad in ["ab", "nome con spazi", "nome@strano", &"x".repeat(51)] {
        let mut body = valid_body();
        body["username"] = json!(bad);
        let (status, body) = post_register(&app, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "username {bad:?}");
        assert_eq!(body["code"], "VALIDATION_FAILED");
        assert!(body["error"].as_str().unwrap().contains("username"));
    }
}

#[sqlx::test]
async fn register_duplicate_email_is_409_email_taken(pool: PgPool) {
    let app = common::test_app(pool);
    app.register("dup@example.com", "primo", "athlete").await;

    let mut body = valid_body();
    body["email"] = json!("dup@example.com");
    body["username"] = json!("secondo");
    let (status, body) = post_register(&app, body).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "EMAIL_TAKEN");
}

#[sqlx::test]
async fn register_duplicate_username_is_409_username_taken(pool: PgPool) {
    let app = common::test_app(pool);
    app.register("uno@example.com", "stesso", "athlete").await;

    let mut body = valid_body();
    body["email"] = json!("due@example.com");
    body["username"] = json!("stesso");
    let (status, body) = post_register(&app, body).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "USERNAME_TAKEN");
}

#[sqlx::test]
async fn login_wrong_password_is_401_invalid_credentials(pool: PgPool) {
    let app = common::test_app(pool);
    app.register("a@example.com", "atleta", "athlete").await;
    let (status, body) = app
        .request(
            Method::POST,
            "/api/auth/login",
            None,
            Some(json!({"email": "a@example.com", "password": "sbagliata1"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "INVALID_CREDENTIALS");
}

#[sqlx::test]
async fn login_unknown_email_is_401_invalid_credentials(pool: PgPool) {
    let app = common::test_app(pool);
    let (status, body) = app
        .request(
            Method::POST,
            "/api/auth/login",
            None,
            Some(json!({"email": "nessuno@example.com", "password": "password123"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "INVALID_CREDENTIALS");
}

#[sqlx::test]
async fn login_ok_returns_role(pool: PgPool) {
    let app = common::test_app(pool);
    app.register("c@example.com", "coach1", "coach").await;
    let (status, body) = app
        .request(
            Method::POST,
            "/api/auth/login",
            None,
            Some(json!({"email": "c@example.com", "password": common::TEST_PASSWORD})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["user"]["role"], "coach");
    assert!(body["token"].as_str().is_some_and(|t| !t.is_empty()));
}

#[sqlx::test]
async fn login_disabled_account_is_401_account_disabled(pool: PgPool) {
    let app = common::test_app(pool.clone());
    let (id, _) = app
        .register("d@example.com", "disabilitato", "athlete")
        .await;
    sqlx::query!("UPDATE users SET is_active = FALSE WHERE id = $1", id)
        .execute(&pool)
        .await
        .unwrap();

    let login = |password: &'static str| {
        app.request(
            Method::POST,
            "/api/auth/login",
            None,
            Some(json!({"email": "d@example.com", "password": password})),
        )
    };

    let (status, body) = login(common::TEST_PASSWORD).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "ACCOUNT_DISABLED");

    // con password errata non si deve rivelare lo stato dell'account
    let (_, body) = login("sbagliata1").await;
    assert_eq!(body["code"], "INVALID_CREDENTIALS");
}

/// Il token di un utente eliminato non dà più accesso: 401, mai 500.
#[sqlx::test]
async fn deleted_user_token_is_401(pool: PgPool) {
    let app = common::test_app(pool);
    let (id, token) = app.register("a@example.com", "atleta", "athlete").await;
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(id)
        .execute(&app.pool)
        .await
        .unwrap();

    let (status, body) = app
        .request(Method::GET, "/api/users/me", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["code"], "UNAUTHORIZED");

    let (status, body) = app
        .upload(&token, &[("run.gpx", b"<gpx/>".to_vec())])
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["code"], "UNAUTHORIZED");
}

/// Il token di un utente disattivato non dà più accesso.
#[sqlx::test]
async fn inactive_user_token_is_401(pool: PgPool) {
    let app = common::test_app(pool);
    let (id, token) = app.register("a@example.com", "atleta", "athlete").await;
    sqlx::query("UPDATE users SET is_active = false WHERE id = $1")
        .bind(id)
        .execute(&app.pool)
        .await
        .unwrap();

    for uri in ["/api/users/me", "/api/activities", "/api/imports"] {
        let (status, body) = app.request(Method::GET, uri, Some(&token), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}: {body}");
        assert_eq!(body["code"], "UNAUTHORIZED", "{uri}");
    }
}
