-- migrations/002_create_users_info.sql
-- Crea la tabella users_info con relazione 1-a-1 verso users.
--
-- Scelta di design:
--   - id BIGSERIAL: chiave primaria propria della riga
--   - user_id BIGINT UNIQUE NOT NULL: FK verso users.id
--     UNIQUE garantisce la relazione 1-a-1 (un solo profilo per utente)
--   - ON DELETE CASCADE: se l'utente viene eliminato, il profilo sparisce con lui

CREATE TABLE IF NOT EXISTS users_info (
    id          BIGSERIAL    PRIMARY KEY,
    user_id     BIGINT       NOT NULL UNIQUE
                             REFERENCES users(id) ON DELETE CASCADE ON UPDATE CASCADE,
    bio         TEXT         NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_users_info_user_id ON users_info(user_id);
