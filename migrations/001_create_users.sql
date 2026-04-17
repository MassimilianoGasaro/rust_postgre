-- migrations/001_create_users.sql
-- Crea la tabella users con tutti i campi necessari all'autenticazione.

CREATE TABLE IF NOT EXISTS users (
    id            BIGSERIAL PRIMARY KEY,
    email         VARCHAR(255) NOT NULL UNIQUE,
    username      VARCHAR(100) NOT NULL UNIQUE,
    password_hash TEXT         NOT NULL,
    is_active     BOOLEAN      NOT NULL DEFAULT TRUE,
    created_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

-- Indice sulla email per velocizzare il login (find_by_email)
CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);
