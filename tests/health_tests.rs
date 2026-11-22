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
