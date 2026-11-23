-- Phase 4: customers/suppliers (parties) and the money ledger.
-- Money is always stored as integer paisa (1 rupee = 100 paisa), never as floats.

CREATE TABLE parties (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id      UUID        NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    name             TEXT        NOT NULL,
    -- Lower-cased, spelling-variant-free copy of the name used for matching.
    normalized_name  TEXT        NOT NULL,
    -- Positive: the party owes us. Negative: we owe the party.
    balance_paisa    BIGINT      NOT NULL DEFAULT 0,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (business_id, normalized_name)
);

CREATE INDEX idx_parties_business_recent ON parties (business_id, updated_at DESC);

CREATE TABLE transactions (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id   UUID        NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    party_id      UUID        REFERENCES parties(id) ON DELETE SET NULL,
    user_id       UUID        NOT NULL REFERENCES users(id),
    kind          TEXT        NOT NULL
                  CHECK (kind IN ('sale', 'expense', 'payment_received', 'payment_made')),
    amount_paisa  BIGINT      NOT NULL CHECK (amount_paisa > 0),
    on_credit     BOOLEAN     NOT NULL DEFAULT FALSE,
    description   TEXT,
    transcript    TEXT,
    -- The business day in Pakistan time, not the UTC date.
    occurred_on   DATE        NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_transactions_business_day ON transactions (business_id, occurred_on);
CREATE INDEX idx_transactions_party ON transactions (party_id, occurred_on DESC);