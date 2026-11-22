-- migrations/0003_activities_imports.sql
-- Attività importate dai file e tracciamento degli import (upload in background).

CREATE TABLE activities (
    id                 BIGSERIAL        PRIMARY KEY,
    user_id            BIGINT           NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    source             TEXT             NOT NULL DEFAULT 'file_upload'
                                        CHECK (source IN ('file_upload', 'garmin', 'coros', 'strava')),
    sport              TEXT             NOT NULL,
    sport_raw          TEXT             NOT NULL,
    name               TEXT,
    start_time         TIMESTAMPTZ      NOT NULL,
    utc_offset_seconds INTEGER,
    elapsed_s          INTEGER          NOT NULL,
    moving_s           INTEGER,
    distance_m         DOUBLE PRECISION,
    elevation_gain_m   DOUBLE PRECISION,
    avg_hr             INTEGER,
    max_hr             INTEGER,
    avg_speed_mps      DOUBLE PRECISION,
    avg_power_w        INTEGER,
    avg_cadence        INTEGER,
    calories           INTEGER,
    file_format        TEXT             NOT NULL,
    file_path          TEXT             NOT NULL,
    file_sha256        TEXT             NOT NULL,
    created_at         TIMESTAMPTZ      NOT NULL DEFAULT NOW(),
    -- Lo stesso file (stesso contenuto) non può essere importato due volte dallo stesso utente.
    -- Il nome esplicito permette al repository di riconoscere la violazione.
    CONSTRAINT activities_user_sha_unique UNIQUE (user_id, file_sha256)
);

-- Elenchi per periodo e ricerca dei duplicati temporali (±60 s).
CREATE INDEX activities_user_start_idx ON activities(user_id, start_time);

-- Un import = un upload (uno o più file, anche ZIP), elaborato in background.
CREATE TABLE imports (
    id          BIGSERIAL   PRIMARY KEY,
    user_id     BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status      TEXT        NOT NULL CHECK (status IN ('processing', 'completed', 'failed')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    finished_at TIMESTAMPTZ
);

CREATE INDEX imports_user_created_idx ON imports(user_id, created_at DESC);

-- Esito di ogni singolo file di un import (i file dentro uno ZIP compresi).
CREATE TABLE import_items (
    id          BIGSERIAL PRIMARY KEY,
    import_id   BIGINT    NOT NULL REFERENCES imports(id) ON DELETE CASCADE,
    filename    TEXT      NOT NULL,
    status      TEXT      NOT NULL CHECK (status IN ('pending', 'imported', 'duplicate', 'failed')),
    activity_id BIGINT    REFERENCES activities(id) ON DELETE SET NULL,
    error_code  TEXT
);

CREATE INDEX import_items_import_idx ON import_items(import_id);
