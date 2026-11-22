//! tests/common/mod.rs — Harness condiviso per i test di integrazione
//!
//! Costruisce il Router reale con un pool fornito da `#[sqlx::test]`
//! (database isolato per ogni test, migration già applicate) e permette
//! di inviare richieste senza aprire socket (tramite `tower::ServiceExt::oneshot`).

#![allow(dead_code)] // ogni file di test usa solo una parte degli helper

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use rust_postgre::{AppState, build_router};
use sqlx::PgPool;
use tower::ServiceExt;

/// Segreto JWT fisso, usato solo nei test.
pub const TEST_JWT_SECRET: &str = "test-secret-test-secret-test-secret-0123";

/// Boundary fisso dei corpi multipart costruiti nei test.
const MULTIPART_BOUNDARY: &str = "----rust-postgre-test-boundary";

/// Password usata da `TestApp::register`.
pub const TEST_PASSWORD: &str = "password123";

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
        // Sopra il limite di default di axum (2 MiB): i test di upload lo superano
        max_file_bytes: 8 * 1024 * 1024,
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

    /// Registra un utente con `TEST_PASSWORD` e ritorna `(id, token)`.
    pub async fn register(&self, email: &str, username: &str, role: &str) -> (i64, String) {
        let (status, body) = self
            .request(
                Method::POST,
                "/api/auth/register",
                None,
                Some(serde_json::json!({
                    "email": email,
                    "username": username,
                    "password": TEST_PASSWORD,
                    "role": role,
                })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "registrazione fallita: {body}");

        let id = body["user"]["id"].as_i64().expect("id utente mancante");
        let token = body["token"].as_str().expect("token mancante").to_string();
        (id, token)
    }

    /// `POST /api/imports` con un campo multipart `files` per ogni elemento
    /// `(nome file, contenuto)`; ritorna status e corpo JSON.
    pub async fn upload(
        &self,
        token: &str,
        files: &[(&str, Vec<u8>)],
    ) -> (StatusCode, serde_json::Value) {
        let parts: Vec<(&str, &str, &[u8])> = files
            .iter()
            .map(|(name, bytes)| ("files", *name, bytes.as_slice()))
            .collect();
        self.upload_fields(token, &parts).await
    }

    /// Come `upload`, ma con nomi di campo arbitrari: `(campo, nome file, contenuto)`.
    pub async fn upload_fields(
        &self,
        token: &str,
        fields: &[(&str, &str, &[u8])],
    ) -> (StatusCode, serde_json::Value) {
        let mut body = Vec::new();
        for (field, filename, bytes) in fields {
            body.extend_from_slice(
                format!(
                    "--{MULTIPART_BOUNDARY}\r\nContent-Disposition: form-data; name=\"{field}\"; \
                     filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
                )
                .as_bytes(),
            );
            body.extend_from_slice(bytes);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{MULTIPART_BOUNDARY}--\r\n").as_bytes());

        let request = Request::builder()
            .method(Method::POST)
            .uri("/api/imports")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={MULTIPART_BOUNDARY}"),
            )
            .body(Body::from(body))
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
        let value = serde_json::from_slice(&bytes).expect("il corpo non è JSON valido");
        (status, value)
    }

    /// Attende la fine dell'import (poll ogni 50 ms, max 10 s) e ne ritorna il dettaglio.
    ///
    /// Panica se l'import non arriva a `completed` entro il tempo massimo.
    pub async fn wait_import(&self, token: &str, id: i64) -> serde_json::Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let (status, body) = self
                .request(
                    Method::GET,
                    &format!("/api/imports/{id}"),
                    Some(token),
                    None,
                )
                .await;
            assert_eq!(status, StatusCode::OK, "lettura import fallita: {body}");
            if body["status"] == "completed" {
                return body;
            }
            if body["status"] != "processing" || std::time::Instant::now() >= deadline {
                panic!("import {id} non completato: {body}");
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
}
