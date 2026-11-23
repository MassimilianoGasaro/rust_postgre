# Immagine di produzione del backend (usata da Render).
# Le query sqlx sono verificate con la cache offline .sqlx/: la build non richiede un database.

# ── Build ────────────────────────────────────────────────────────────────────
FROM rust:1-slim-bookworm AS build
WORKDIR /src
ENV SQLX_OFFLINE=true
COPY Cargo.toml Cargo.lock ./
COPY .sqlx ./.sqlx
COPY migrations ./migrations
COPY src ./src
RUN cargo build --release --locked --bin rust_postgre

# ── Runtime ──────────────────────────────────────────────────────────────────
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && apt-get clean \
    && useradd --system --create-home --home-dir /app app
WORKDIR /app
COPY --from=build /src/target/release/rust_postgre /usr/local/bin/rust_postgre
USER app

# Dietro al proxy della piattaforma: HTTP semplice su tutte le interfacce.
# Render imposta PORT (che ha la precedenza su SERVER_PORT).
ENV SERVER_HOST=0.0.0.0 \
    SERVER_PORT=8080 \
    TLS_ENABLED=false \
    UPLOAD_DIR=/app/uploads
EXPOSE 8080
CMD ["rust_postgre"]
