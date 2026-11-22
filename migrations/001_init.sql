-- Phase 1: businesses and the people who talk to the bot

CREATE TABLE businesses (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT        NOT NULL,
    language    TEXT        NOT NULL DEFAULT 'ur',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE users (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id  UUID        NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    phone        TEXT        NOT NULL UNIQUE,
    name         TEXT        NOT NULL,
    role         TEXT        NOT NULL CHECK (role IN ('owner', 'employee')),
    language     TEXT        NOT NULL DEFAULT 'ur',
    is_active    BOOLEAN     NOT NULL DEFAULT TRUE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_users_business ON users (business_id);