pub mod access_service;
pub mod activity_service;
/// services/mod.rs — Entry point del layer service
///
/// I service contengono la LOGICA DI BUSINESS dell'applicazione:
/// - hash delle password
/// - generazione e validazione JWT
/// - validazioni di business (es. email già esistente)
/// - orchestrazione di più repository
///
/// NON conoscono HTTP — non usano Request/Response di Axum.
/// Questo li rende riutilizzabili e testabili indipendentemente.
pub mod auth_service;
pub mod coach_link_service;
pub mod import_service;
pub mod stats_service;
pub mod user_service;
