/// controllers/mod.rs — Espone i moduli controller
///
/// Gli handler HTTP vivono qui. Ogni controller gestisce
/// un "gruppo" di endpoint logicamente correlati.

pub mod auth_controller;
pub mod health_controller;
pub mod user_controller;
pub mod coach_link_controller;
pub mod import_controller;
pub mod activity_controller;
