mod common;

use axum::http::{Method, StatusCode};
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test]
async fn health_returns_ok(pool: PgPool) {
    let app = common::test_app(pool);
    let (status, body) = app.request(Method::GET, "/api/health", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"status": "ok"}));
}

/// Una route inesistente risponde 404 nel formato d'errore JSON.
#[sqlx::test]
async fn unknown_route_is_404_json(pool: PgPool) {
    let app = common::test_app(pool);
    for uri in ["/api/does-not-exist", "/does-not-exist", "/api/users/me/extra"] {
        let (status, body) = app.request(Method::GET, uri, None, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        assert_eq!(body["code"], "NOT_FOUND", "{uri}");
        assert_eq!(body["status"], 404, "{uri}");
        assert!(body["error"].is_string(), "{uri}");
    }
}

/// Un metodo non previsto su una route esistente risponde 405 JSON.
#[sqlx::test]
async fn wrong_method_is_405_json(pool: PgPool) {
    let app = common::test_app(pool);
    let (status, body) = app
        .request(Method::DELETE, "/api/health", None, None)
        .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(body["code"], "METHOD_NOT_ALLOWED");
    assert_eq!(body["status"], 405);
}
