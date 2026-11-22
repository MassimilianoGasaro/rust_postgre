mod common;

use axum::http::{Method, StatusCode};
use rust_postgre::repositories::user_repository;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test]
async fn me_requires_token(pool: PgPool) {
    let app = common::test_app(pool);
    let (status, body) = app.request(Method::GET, "/api/users/me", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "UNAUTHORIZED");
}

#[sqlx::test]
async fn me_returns_default_profile(pool: PgPool) {
    let app = common::test_app(pool);
    let (id, token) = app.register("a@example.com", "atleta", "athlete").await;
    let (status, body) = app
        .request(Method::GET, "/api/users/me", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], id);
    assert_eq!(body["email"], "a@example.com");
    assert_eq!(body["username"], "atleta");
    assert_eq!(body["role"], "athlete");
    assert!(body["created_at"].is_string());
    assert!(body.get("password_hash").is_none());
    assert_eq!(body["profile"]["language"], "it");
    assert_eq!(body["profile"]["timezone"], "Europe/Rome");
    assert!(body["profile"]["weight_kg"].is_null());
    assert!(body["profile"]["birth_date"].is_null());
}

#[sqlx::test]
async fn patch_me_updates_profile_fields(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = app.register("a@example.com", "atleta", "athlete").await;
    let (status, body) = app
        .request(
            Method::PATCH,
            "/api/users/me",
            Some(&token),
            Some(json!({
                "weight_kg": 70.5,
                "max_hr": 190,
                "birth_date": "1990-05-17",
                "sex": "female",
                "language": "en"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["profile"]["weight_kg"], 70.5);

    let (_, body) = app
        .request(Method::GET, "/api/users/me", Some(&token), None)
        .await;
    assert_eq!(body["profile"]["weight_kg"], 70.5);
    assert_eq!(body["profile"]["max_hr"], 190);
    assert_eq!(body["profile"]["birth_date"], "1990-05-17");
    assert_eq!(body["profile"]["sex"], "female");
    assert_eq!(body["profile"]["language"], "en");
    // i campi non inviati restano invariati
    assert_eq!(body["profile"]["timezone"], "Europe/Rome");
}

#[sqlx::test]
async fn patch_me_null_clears_field_and_absent_keeps_it(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = app.register("a@example.com", "atleta", "athlete").await;
    app.request(
        Method::PATCH,
        "/api/users/me",
        Some(&token),
        Some(json!({"weight_kg": 70.5, "max_hr": 190})),
    )
    .await;

    let (status, body) = app
        .request(
            Method::PATCH,
            "/api/users/me",
            Some(&token),
            Some(json!({"weight_kg": null})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["profile"]["weight_kg"].is_null());
    assert_eq!(body["profile"]["max_hr"], 190);
}

#[sqlx::test]
async fn patch_me_updates_account_fields_and_password(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = app.register("a@example.com", "atleta", "athlete").await;
    let (status, body) = app
        .request(
            Method::PATCH,
            "/api/users/me",
            Some(&token),
            Some(json!({
                "username": "nuovo.nome",
                "password": "nuovapassword1",
                "current_password": common::TEST_PASSWORD,
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["username"], "nuovo.nome");

    let (status, _) = app
        .request(
            Method::POST,
            "/api/auth/login",
            None,
            Some(json!({"email": "a@example.com", "password": "nuovapassword1"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

/// Cambiare password o email richiede la password attuale.
#[sqlx::test]
async fn patch_me_sensitive_fields_require_current_password(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = app.register("a@example.com", "atleta", "athlete").await;

    for change in [
        json!({"password": "nuovapassword1"}),
        json!({"email": "b@example.com"}),
    ] {
        // Assente → 400
        let (status, body) = app
            .request(
                Method::PATCH,
                "/api/users/me",
                Some(&token),
                Some(change.clone()),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{change}");
        assert_eq!(body["code"], "VALIDATION_FAILED", "{change}");

        // Errata → 401
        let mut wrong = change.clone();
        wrong["current_password"] = json!("sbagliata123");
        let (status, body) = app
            .request(Method::PATCH, "/api/users/me", Some(&token), Some(wrong))
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{change}");
        assert_eq!(body["code"], "INVALID_CREDENTIALS", "{change}");
    }

    // Nessuna modifica applicata: email e password sono quelle iniziali
    let (status, body) = app
        .request(Method::GET, "/api/users/me", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["email"], "a@example.com");
    let (status, _) = app
        .request(
            Method::POST,
            "/api/auth/login",
            None,
            Some(json!({"email": "a@example.com", "password": common::TEST_PASSWORD})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Con la password attuale corretta l'email cambia
    let (status, body) = app
        .request(
            Method::PATCH,
            "/api/users/me",
            Some(&token),
            Some(json!({"email": "b@example.com", "current_password": common::TEST_PASSWORD})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["email"], "b@example.com");
}

#[sqlx::test]
async fn patch_me_conflicting_username_is_409(pool: PgPool) {
    let app = common::test_app(pool);
    app.register("b@example.com", "occupato", "athlete").await;
    let (_, token) = app.register("a@example.com", "atleta", "athlete").await;
    let (status, body) = app
        .request(
            Method::PATCH,
            "/api/users/me",
            Some(&token),
            Some(json!({"username": "occupato"})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "USERNAME_TAKEN");
}

#[sqlx::test]
async fn patch_me_invalid_timezone_is_400(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = app.register("a@example.com", "atleta", "athlete").await;
    let (status, body) = app
        .request(
            Method::PATCH,
            "/api/users/me",
            Some(&token),
            Some(json!({"timezone": "Mars/Base"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "INVALID_TIMEZONE");
}

#[sqlx::test]
async fn patch_me_valid_timezone_is_stored_and_readable_by_timezone_of(pool: PgPool) {
    let app = common::test_app(pool.clone());
    let (id, token) = app.register("a@example.com", "atleta", "athlete").await;
    let (status, body) = app
        .request(
            Method::PATCH,
            "/api/users/me",
            Some(&token),
            Some(json!({"timezone": "America/New_York"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["profile"]["timezone"], "America/New_York");
    assert_eq!(
        user_repository::timezone_of(&pool, id).await.unwrap(),
        "America/New_York"
    );
}

#[sqlx::test]
async fn patch_me_validation_errors_are_400(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = app.register("a@example.com", "atleta", "athlete").await;
    for bad in [
        json!({"language": "fr"}),
        json!({"password": "corta"}),
        json!({"sex": "robot"}),
        json!({"max_hr": "tanti"}),
        json!({"weight_kg": -3.0}),
    ] {
        let (status, body) = app
            .request(
                Method::PATCH,
                "/api/users/me",
                Some(&token),
                Some(bad.clone()),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(body["code"], "VALIDATION_FAILED", "{bad}");
    }
}

#[sqlx::test]
async fn old_users_routes_are_gone(pool: PgPool) {
    let app = common::test_app(pool);
    let (id, token) = app.register("a@example.com", "atleta", "athlete").await;
    let (status, _) = app
        .request(Method::GET, &format!("/api/users/{id}"), Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app
        .request(Method::GET, "/api/users", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn delete_me_then_login_fails(pool: PgPool) {
    let app = common::test_app(pool.clone());
    let (id, token) = app.register("a@example.com", "atleta", "athlete").await;

    // cartella upload dell'utente: deve sparire insieme all'account
    let user_dir = app.upload_dir.path().join(id.to_string());
    std::fs::create_dir_all(&user_dir).unwrap();
    std::fs::write(user_dir.join("file.fit"), b"dati").unwrap();

    let (status, _) = app
        .request(
            Method::DELETE,
            "/api/users/me",
            Some(&token),
            Some(json!({"current_password": common::TEST_PASSWORD})),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(!user_dir.exists());

    let (status, _) = app
        .request(
            Method::POST,
            "/api/auth/login",
            None,
            Some(json!({"email": "a@example.com", "password": common::TEST_PASSWORD})),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // il profilo è stato eliminato in cascata
    let n = sqlx::query_scalar!("SELECT COUNT(*) FROM user_profiles WHERE user_id = $1", id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, Some(0));
}

#[sqlx::test]
async fn delete_me_without_upload_dir_is_204(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = app.register("a@example.com", "atleta", "athlete").await;
    let (status, _) = app
        .request(
            Method::DELETE,
            "/api/users/me",
            Some(&token),
            Some(json!({"current_password": common::TEST_PASSWORD})),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

/// L'eliminazione dell'account richiede la password attuale.
#[sqlx::test]
async fn delete_me_requires_current_password(pool: PgPool) {
    let app = common::test_app(pool);
    let (_, token) = app.register("a@example.com", "atleta", "athlete").await;

    for body in [None, Some(json!({})), Some(json!({"current_password": 5}))] {
        let (status, resp) = app
            .request(Method::DELETE, "/api/users/me", Some(&token), body.clone())
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
        assert_eq!(resp["code"], "VALIDATION_FAILED", "{body:?}");
    }

    let (status, resp) = app
        .request(
            Method::DELETE,
            "/api/users/me",
            Some(&token),
            Some(json!({"current_password": "sbagliata123"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(resp["code"], "INVALID_CREDENTIALS");

    // L'account esiste ancora
    let (status, _) = app
        .request(Method::GET, "/api/users/me", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::OK);
}
