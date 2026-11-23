-- Phase 5: tasks the owner gives to employees.

CREATE TABLE tasks (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id   UUID        NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    assigned_by   UUID        NOT NULL REFERENCES users(id),
    assignee_id   UUID        NOT NULL REFERENCES users(id),
    title         TEXT        NOT NULL,
    status        TEXT        NOT NULL DEFAULT 'open'
                  CHECK (status IN ('open', 'done', 'cancelled')),
    due_date      DATE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at  TIMESTAMPTZ
);

CREATE INDEX idx_tasks_business_status ON tasks (business_id, status, due_date);
CREATE INDEX idx_tasks_assignee_status ON tasks (assignee_id, status);