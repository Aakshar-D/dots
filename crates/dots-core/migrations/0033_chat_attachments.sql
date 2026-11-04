-- Attachments in chat (attachments spec section 4.1): a file the person added to a chat box.
-- Dots keeps its own copy under <data>/chat-files/<id>/<file_name>. It is staged (run_id and
-- scheduled_id NULL) until a message takes it: a chat run (run_id, with its name in the topic's
-- attachments folder) or a scheduled message (scheduled_id). Sending clears scheduled_id in the
-- same write that sets run_id.
CREATE TABLE chat_attachments (
    id           TEXT PRIMARY KEY,
    dot_id       TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    run_id       TEXT REFERENCES runs(id) ON DELETE CASCADE,
    scheduled_id TEXT REFERENCES scheduled_messages(id) ON DELETE CASCADE,
    file_name    TEXT NOT NULL,
    name         TEXT,
    size         INTEGER NOT NULL,
    created_at   TEXT NOT NULL,
    CHECK (run_id IS NULL OR name IS NOT NULL)
);
CREATE INDEX chat_attachments_run ON chat_attachments(run_id);
CREATE INDEX chat_attachments_scheduled ON chat_attachments(scheduled_id);
