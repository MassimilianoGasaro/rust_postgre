//! tests/common/mod.rs — Harness condiviso per i test di integrazione
//!
//! Costruisce il Router reale con un pool fornito da `#[sqlx::test]`
//! (database isolato per ogni test, migration già applicate) e permette
//! di inviare richieste senza aprire socket (tramite `tower::ServiceExt::oneshot`).

#![allow(dead_code)] // ogni file di test usa solo una parte degli helper

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use rust_postgre::{build_router, AppState};
use sqlx::PgPool;
use tower::ServiceExt;

/// Segreto JWT fisso, usato solo nei test.
pub const TEST_JWT_SECRET: &str = "test-secret-test-secret-test-secret-0123";

/// Applicazione di test: router + risorse che devono restare vive per tutto il test.
pub struct TestApp {
    pub router: Router,
    /// Directory upload temporanea — viene eliminata al drop di `TestApp`.
    pub upload_dir: tempfile::TempDir,
    pub pool: PgPool,
}

/// Costruisce l'app di test sul pool indicato.
pub fn test_app(pool: PgPool) -> TestApp {
    let upload_dir = tempfile::tempdir().expect("impossibile creare la directory temporanea");

    let state = AppState {
        pool: pool.clone(),
        jwt_secret: TEST_JWT_SECRET.to_string(),
        jwt_expiration_hours: 1,
        upload_dir: upload_dir.path().to_path_buf(),
        max_upload_bytes: 10 * 1024 * 1024,
        max_file_bytes: 1024 * 1024,
        cors_origin: None,
    };

    TestApp {
        router: build_router(state),
        upload_dir,
        pool,
    }
}

impl TestApp {
    /// Invia una richiesta JSON al router.
    ///
    /// Ritorna lo status e il corpo JSON; se il corpo è vuoto → `Value::Null`.
    pub async fn request(
        &self,
        method: Method,
        uri: &str,
        token: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder().method(method).uri(uri);

        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }

        let request = match body {
            Some(json) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json.to_string())),
            None => builder.body(Body::empty()),
        }
        .expect("richiesta di test non valida");

        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("il router è infallibile");

        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("lettura del corpo fallita")
            .to_bytes();

        let value = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).expect("il corpo non è JSON valido")
        };

        (status, value)
    }
}
