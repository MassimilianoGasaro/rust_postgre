-- migrations/0001_users.sql
-- Schema iniziale: utenti (atleta/coach) e profilo fisiologico 1-a-1.

CREATE TABLE users (
    id            BIGSERIAL    PRIMARY KEY,
    email         VARCHAR(255) NOT NULL UNIQUE,
    username      VARCHAR(50)  NOT NULL UNIQUE,
    password_hash TEXT         NOT NULL,
    role          TEXT         NOT NULL CHECK (role IN ('athlete', 'coach')),
    is_active     BOOLEAN      NOT NULL DEFAULT TRUE,
    created_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

-- Profilo utente (1-a-1): preferenze e parametri fisiologici.
-- ON DELETE CASCADE: se l'utente viene eliminato, il profilo sparisce con lui.
CREATE TABLE user_profiles (
    user_id                 BIGINT           PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    language                TEXT             NOT NULL DEFAULT 'it' CHECK (language IN ('it', 'en')),
    timezone                TEXT             NOT NULL DEFAULT 'Europe/Rome',
    birth_date              DATE,
    sex                     TEXT             CHECK (sex IN ('male', 'female', 'other')),
    weight_kg               DOUBLE PRECISION,
    max_hr                  INTEGER,
    resting_hr              INTEGER,
    ftp_watts               INTEGER,
    threshold_pace_s_per_km INTEGER
);
