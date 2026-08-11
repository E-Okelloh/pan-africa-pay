-- 001_init.sql
-- Initial schema for Pan-Africa Pay: users, wallets, payments, idempotency keys.
--
-- Design notes:
-- * Monetary values are stored as BIGINT minor units (cents for KES,
--   avos for USDC) to avoid floating-point drift.
-- * Enums are stored as TEXT with CHECK constraints for readability
--   and simpler future migrations.
-- * created_at/updated_at are managed by the application for updated_at
--   and defaults for created_at, keeping triggers out of the way.

-- ---------------------------------------------------------------------------
-- users
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS users (
    id            UUID PRIMARY KEY,
    email         TEXT UNIQUE NOT NULL,
    phone         TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    kyc_tier      TEXT NOT NULL DEFAULT 'NONE',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_users_phone ON users (phone);

-- ---------------------------------------------------------------------------
-- wallets
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS wallets (
    id         UUID PRIMARY KEY,
    user_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    currency   TEXT NOT NULL CHECK (currency IN ('KES', 'USDC')),
    balance    BIGINT NOT NULL DEFAULT 0 CHECK (balance >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (user_id, currency)
);

CREATE INDEX IF NOT EXISTS idx_wallets_user ON wallets (user_id);

-- ---------------------------------------------------------------------------
-- payments
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS payments (
    id                       UUID PRIMARY KEY,
    user_id                  UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    payment_type             TEXT NOT NULL CHECK (payment_type IN ('COLLECT', 'PAYOUT', 'DEPOSIT', 'WITHDRAW')),
    rail                     TEXT NOT NULL CHECK (rail IN ('MPESA', 'KOTANI')),
    status                   TEXT NOT NULL CHECK (status IN ('PENDING', 'PROCESSING', 'COMPLETED', 'FAILED', 'EXPIRED')),
    amount                   BIGINT NOT NULL,
    currency                 TEXT NOT NULL CHECK (currency IN ('KES', 'USDC')),
    fee                      BIGINT NOT NULL DEFAULT 0,
    mpesa_checkout_request_id TEXT,
    mpesa_receipt_number     TEXT,
    kotani_tx_id             TEXT,
    callback_payload         JSONB,
    idempotency_key          TEXT NOT NULL UNIQUE,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT payments_amount_positive CHECK (amount > 0)
);

CREATE INDEX IF NOT EXISTS idx_payments_user_created ON payments (user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_payments_status ON payments (status);
CREATE INDEX IF NOT EXISTS idx_payments_mpesa_checkout ON payments (mpesa_checkout_request_id);
CREATE INDEX IF NOT EXISTS idx_payments_kotani_tx ON payments (kotani_tx_id);

-- ---------------------------------------------------------------------------
-- idempotency_keys
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS idempotency_keys (
    idempotency_key TEXT PRIMARY KEY,
    request_hash    TEXT NOT NULL,
    response_body   JSONB NOT NULL,
    status_code     INT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at      TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_idempotency_expires ON idempotency_keys (expires_at);

-- ---------------------------------------------------------------------------
-- audit_log (append-only event history for financial integrity)
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS audit_log (
    id          BIGSERIAL PRIMARY KEY,
    event_type  TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    entity_id   UUID,
    actor_id    UUID,
    payload     JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_audit_log_entity ON audit_log (entity_type, entity_id);
CREATE INDEX IF NOT EXISTS idx_audit_log_created ON audit_log (created_at DESC);

-- Note: PostgreSQL disallows updating rows in an append-only table
-- via trigger; application code enforces append-only semantics.
