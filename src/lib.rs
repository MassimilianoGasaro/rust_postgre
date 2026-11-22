//! lib.rs — Libreria dell'applicazione
//!
//! Espone moduli, `AppState` e `build_router`. Separare la libreria dal
//! binario (main.rs) permette ai test di integrazione (cartella `tests/`)
//! di costruire il Router reale senza avviare un server TCP.

pub mod config;
pub mod controllers;
pub mod dto;
pub mod errors;
pub mod middleware;
pub mod models;
pub mod repositories;
pub mod services;

use std::path::PathBuf;

use axum::{
    http::{header, HeaderValue, Method},
    middleware::from_fn_with_state,
    routing::{delete, get, patch, post},
    Router,
};
use sqlx::PgPool;
use tower_http::cors::CorsLayer;

use controllers::{auth_controller, health_controller, user_controller};

/// AppState — stato condiviso tra tutti gli handler.
///
/// In Axum lo stato viene clonato per ogni request, quindi deve
/// implementare Clone (PgPool è già un handle economico da clonare).
///
/// Tutti gli handler ricevono questo stato con:
///   `State(state): State<AppState>`
#[derive(Clone)]
pub struct AppState {
    /// Pool di connessioni a PostgreSQL — thread-safe, condiviso
    pub pool: PgPool,
    /// Segreto per firmare/verificare i JWT
    pub jwt_secret: String,
    /// Ore di validità del token JWT
    pub jwt_expiration_hours: i64,
    /// Directory in cui vengono salvati i file caricati
    pub upload_dir: PathBuf,
    /// Dimensione massima del body di una richiesta di upload (byte)
    pub max_upload_bytes: usize,
    /// Dimensione massima di un singolo file caricato (byte)
    pub max_file_bytes: usize,
    /// Origine consentita dal CORS (None → CORS non abilitato)
    pub cors_origin: Option<String>,
}

/// Costruisce e restituisce il Router dell'applicazione.
///
/// Separare questa funzione da main() ci permette di
/// riutilizzarla nei test di integrazione.
///
/// Struttura delle route (tutte sotto `/api`):
///   /api/health → health check pubblico
///   /api/auth   → route pubbliche (no JWT richiesto)
///   /api/users  → route protette (JWT obbligatorio tramite middleware)
pub fn build_router(state: AppState) -> Router {
    // ── Route PUBBLICHE — accessibili senza token ──────────────────────────
    let auth_routes = Router::new()
        .route("/register", post(auth_controller::register))
        // POST /api/auth/register → registra un nuovo utente
        .route("/login", post(auth_controller::login));
    // POST /api/auth/login → autentica e restituisce il JWT

    // ── Route PROTETTE — richiedono JWT valido nell'header ─────────────────
    let user_routes = Router::new()
        .route("/", get(user_controller::list))
        // GET /api/users → lista paginata utenti
        .route("/:id", get(user_controller::get_by_id))
        // GET /api/users/:id → singolo utente
        .route("/:id", patch(user_controller::update))
        // PATCH /api/users/:id → aggiornamento parziale
        .route("/:id", delete(user_controller::delete))
        // DELETE /api/users/:id → elimina utente
        //
        // Applica il middleware JWT a TUTTE le route di questo gruppo.
        // Il middleware viene eseguito PRIMA dell'handler.
        .route_layer(from_fn_with_state(
            state.clone(),
            middleware::auth_middleware,
        ));

    // ── Tutte le route applicative vivono sotto /api ───────────────────────
    let api = Router::new()
        .route("/health", get(health_controller::health))
        .nest("/auth", auth_routes)
        .nest("/users", user_routes);

    let cors_origin = state.cors_origin.clone();

    let router = Router::new().nest("/api", api).with_state(state);

    // ── CORS — solo se è configurata un'origine consentita ─────────────────
    match cors_origin {
        Some(origin) => {
            let origin: HeaderValue = origin
                .parse()
                .expect("CORS_ORIGIN non è un'origine valida");
            router.layer(
                CorsLayer::new()
                    .allow_origin(origin)
                    .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
                    .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]),
            )
        }
        None => router,
    }
}
