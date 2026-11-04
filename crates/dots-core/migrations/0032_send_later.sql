-- Send later (send later spec section 4.1): a chat message the person wrote now, for Dots to
-- send at `due_at` to the topic it was written in. Every change of a row is one conditional
-- UPDATE, so the sender, Send now, Edit and Cancel never both win.
CREATE TABLE scheduled_messages (
    id          TEXT PRIMARY KEY,
    dot_id      TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    root_run_id TEXT,              -- the topic it was written in; NULL = a new topic
    text        TEXT NOT NULL,
    due_at      TEXT NOT NULL,     -- RFC 3339 UTC
    status      TEXT NOT NULL,     -- scheduled | sending | sent | failed | cancelled
    waiting     TEXT,              -- a due message not sent yet: busy | approval | usage_limit
    error       TEXT,              -- failed: why
    run_id      TEXT REFERENCES runs(id) ON DELETE SET NULL,  -- sent: the chat run
    sent_at     TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX scheduled_messages_due ON scheduled_messages(status, due_at);
CREATE INDEX scheduled_messages_dot ON scheduled_messages(dot_id, status);
