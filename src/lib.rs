//! lib.rs — Libreria dell'applicazione
//!
//! Espone moduli, `AppState` e `build_router`. Separare la libreria dal
//! binario (main.rs) permette ai test di integrazione (cartella `tests/`)
//! di costruire il Router reale senza avviare un server TCP.

pub mod config;
pub mod controllers;
pub mod dto;
pub mod errors;
pub mod extractors;
pub mod middleware;
pub mod models;
pub mod parsing;
pub mod repositories;
pub mod services;
pub mod storage;

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::{HeaderValue, Method, header},
    middleware::from_fn_with_state,
    routing::{delete, get, post},
};
use sqlx::PgPool;
use tokio::sync::Semaphore;
use tower_http::cors::CorsLayer;

use controllers::{
    activity_controller, auth_controller, coach_link_controller, health_controller,
    import_controller, stats_controller, user_controller,
};

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
    /// Permessi per l'elaborazione degli import: al massimo `MAX_CONCURRENT_IMPORTS`
    /// import elaborati in parallelo, gli altri restano `processing` in attesa
    pub import_permits: Arc<Semaphore>,
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
///   /api/coach-links, /api/athletes → route protette (relazione allenatore-atleta)
///   /api/imports → route protette (upload di file di attività ed esito)
///   /api/activities → route protette (elenco e cancellazione attività)
///   /api/stats → route protette (statistiche per intervallo di date)
pub fn build_router(state: AppState) -> Router {
    // ── Route PUBBLICHE — accessibili senza token ──────────────────────────
    let auth_routes = Router::new()
        .route("/register", post(auth_controller::register))
        // POST /api/auth/register → registra un nuovo utente
        .route("/login", post(auth_controller::login));
    // POST /api/auth/login → autentica e restituisce il JWT

    // ── Route PROTETTE — richiedono JWT valido nell'header ─────────────────
    let user_routes = Router::new()
        // GET    /api/users/me → account + profilo dell'utente autenticato
        // PATCH  /api/users/me → aggiornamento parziale (campo assente = invariato)
        // DELETE /api/users/me → elimina il proprio account (204)
        .route(
            "/me",
            get(user_controller::get_me)
                .patch(user_controller::update_me)
                .delete(user_controller::delete_me),
        )
        //
        // Applica il middleware JWT a TUTTE le route di questo gruppo.
        // Il middleware viene eseguito PRIMA dell'handler.
        .route_layer(from_fn_with_state(
            state.clone(),
            middleware::auth_middleware,
        ));

    // Relazione allenatore-atleta (protette da JWT come /users):
    //   POST /api/coach-links              → l'atleta richiede un coach
    //   GET  /api/coach-links?status=      → i propri link / quelli ricevuti
    //   POST /api/coach-links/:id/{accept,reject,cancel,revoke}
    //   GET  /api/athletes                 → atleti collegati al coach
    let coach_link_routes = Router::new()
        .route(
            "/coach-links",
            get(coach_link_controller::list).post(coach_link_controller::create),
        )
        .route(
            "/coach-links/:id/accept",
            post(coach_link_controller::accept),
        )
        .route(
            "/coach-links/:id/reject",
            post(coach_link_controller::reject),
        )
        .route(
            "/coach-links/:id/cancel",
            post(coach_link_controller::cancel),
        )
        .route(
            "/coach-links/:id/revoke",
            post(coach_link_controller::revoke),
        )
        .route("/athletes", get(coach_link_controller::list_athletes))
        .route_layer(from_fn_with_state(
            state.clone(),
            middleware::auth_middleware,
        ));

    // Import di file di attività (protette da JWT):
    //   POST /api/imports      → upload multipart, elaborato in background (202)
    //   GET  /api/imports      → ultimi 20 import dell'utente
    //   GET  /api/imports/:id  → dettaglio con l'esito di ogni file
    //
    // L'upload sostituisce il limite di default di axum (2 MiB) con MAX_UPLOAD_BYTES.
    let import_routes = Router::new()
        .route(
            "/imports",
            // Il limite è applicato solo al POST (registrato prima del layer)
            post(import_controller::upload)
                .layer(DefaultBodyLimit::max(state.max_upload_bytes))
                .get(import_controller::list),
        )
        .route("/imports/:id", get(import_controller::get))
        .route_layer(from_fn_with_state(
            state.clone(),
            middleware::auth_middleware,
        ));

    // Attività (protette da JWT):
    //   GET    /api/activities      → elenco paginato con filtri
    //   DELETE /api/activities/:id  → elimina un'attività propria (204)
    let activity_routes = Router::new()
        .route("/activities", get(activity_controller::list))
        .route("/activities/:id", delete(activity_controller::delete))
        .route_layer(from_fn_with_state(
            state.clone(),
            middleware::auth_middleware,
        ));

    // Statistiche (protette da JWT):
    //   GET /api/stats/summary → totali, per sport e serie per periodo
    let stats_routes = Router::new()
        .route("/stats/summary", get(stats_controller::summary))
        .route_layer(from_fn_with_state(
            state.clone(),
            middleware::auth_middleware,
        ));

    // ── Tutte le route applicative vivono sotto /api ───────────────────────
    let api = Router::new()
        .route("/health", get(health_controller::health))
        .nest("/auth", auth_routes)
        .nest("/users", user_routes)
        .merge(coach_link_routes)
        .merge(import_routes)
        .merge(activity_routes)
        .merge(stats_routes);

    let cors_origin = state.cors_origin.clone();

    let router = Router::new().nest("/api", api).with_state(state);

    // ── CORS — solo se è configurata un'origine consentita ─────────────────
    match cors_origin {
        Some(origin) => {
            let origin: HeaderValue = origin.parse().expect("CORS_ORIGIN non è un'origine valida");
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
