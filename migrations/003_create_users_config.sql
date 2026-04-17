-- migrations/003_create_users_config.sql
-- Crea la tabella users_config con relazione 1-a-1 verso users.
--
-- Scelta di design:
--   - id BIGSERIAL: chiave primaria propria della riga
--   - user_id BIGINT UNIQUE NOT NULL: FK verso users.id
--     UNIQUE garantisce la relazione 1-a-1 (una sola config per utente)
--   - ON DELETE CASCADE: se l'utente viene eliminato, la config sparisce con lui

CREATE TABLE IF NOT EXISTS users_config (
    id          BIGSERIAL    PRIMARY KEY,
    user_id     BIGINT       NOT NULL UNIQUE
                             REFERENCES users(id) ON DELETE CASCADE ON UPDATE CASCADE,
    can_edit    BOOLEAN      NOT NULL DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS idx_users_config_user_id ON users_config(user_id);
