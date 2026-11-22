/// repositories/mod.rs — Entry point del layer repository
///
/// I repository sono l'UNICO punto in cui scriviamo query SQL.
/// Nessun'altra parte dell'app conosce SQL — solo i repository.
/// Vantaggi:
/// - Se cambi DB, cambi solo i repository
/// - Facile da testare (puoi mockare il repository)
/// - Il codice business rimane pulito

pub mod user_repository;
pub mod coach_link_repository;
