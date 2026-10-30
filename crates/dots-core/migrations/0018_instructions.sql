-- Instruction proposals (conversational chat spec section 4): from a person's chat, a dot
-- proposes new instructions for itself; the person approves or rejects them on a card. `base`
-- is the dot's instructions when it proposed, so an approval after the person edited them is
-- refused. One proposal waits per dot: a newer one replaces it.
CREATE TABLE instruction_proposals (
    id            TEXT PRIMARY KEY,
    dot_id        TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    run_id        TEXT NOT NULL,
    root_run_id   TEXT NOT NULL,
    base          TEXT NOT NULL,
    instructions  TEXT NOT NULL,
    reason        TEXT NOT NULL,
    status        TEXT NOT NULL,
    answer_run_id TEXT,
    created_at    TEXT NOT NULL,
    decided_at    TEXT
);
CREATE INDEX instruction_proposals_dot ON instruction_proposals(dot_id, created_at);
CREATE UNIQUE INDEX instruction_proposals_pending_dot
    ON instruction_proposals(dot_id) WHERE status = 'pending';

-- The dots whose policy gets the `Instructions` rule its preset has, Custom too
-- (`upgrade::finish_0018`).
CREATE TABLE upgrade_0018 (
    dot_id TEXT PRIMARY KEY REFERENCES dots(id) ON DELETE CASCADE
);
INSERT INTO upgrade_0018 (dot_id) SELECT id FROM dots;
