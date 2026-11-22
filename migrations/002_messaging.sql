-- Phase 2: webhook de-duplication and the confirm-before-save queue

-- Meta retries webhook deliveries, so every WhatsApp message id is processed exactly once.
CREATE TABLE processed_messages (
    message_id   TEXT        PRIMARY KEY,
    received_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Every money or task action is read back to the user and waits here for a Yes/No tap.
CREATE TABLE pending_actions (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id  UUID        NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    user_id      UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    payload      JSONB       NOT NULL,
    summary      TEXT        NOT NULL,
    transcript   TEXT        NOT NULL DEFAULT '',
    status       TEXT        NOT NULL DEFAULT 'pending'
                 CHECK (status IN ('pending', 'confirmed', 'cancelled', 'expired')),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at   TIMESTAMPTZ NOT NULL DEFAULT (now() + interval '15 minutes')
);

CREATE INDEX idx_pending_user_status ON pending_actions (user_id, status);