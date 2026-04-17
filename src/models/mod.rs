/// models/mod.rs — Espone i modelli del database
///
/// I models rispecchiano le tabelle nel DB.
/// Vengono usati dai repository per mappare le righe SQL in struct Rust.
/// NON vengono mai esposti direttamente nelle risposte HTTP — per quello ci sono i DTO.

pub mod user;
