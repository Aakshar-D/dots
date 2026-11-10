-- All Claude use (usage tab spec section 8): what Claude Code's own transcripts say, read
-- incrementally. Each transcript file read so far, and the byte offset read up to.
CREATE TABLE claude_transcript_files (
    path        TEXT PRIMARY KEY,
    size        INTEGER NOT NULL,
    mtime_ms    INTEGER NOT NULL,
    offset      INTEGER NOT NULL,
    scanned_at  TEXT NOT NULL
);

-- One assistant message, once per (message id, request id) across all files. `dots` is 1 when
-- the message was Dots' when it was read (its cwd in Dots' data folder, or its session on a run).
CREATE TABLE claude_messages (
    message_id             TEXT NOT NULL,
    request_id             TEXT NOT NULL,
    ts                     TEXT NOT NULL,
    session_id             TEXT NOT NULL,
    cwd                    TEXT NOT NULL,
    model                  TEXT NOT NULL,
    input_tokens           INTEGER NOT NULL,
    cache_creation_tokens  INTEGER NOT NULL,
    cache_read_tokens      INTEGER NOT NULL,
    output_tokens          INTEGER NOT NULL,
    dots                   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (message_id, request_id)
) WITHOUT ROWID;

CREATE INDEX claude_messages_ts ON claude_messages(ts);
CREATE INDEX IF NOT EXISTS runs_session_id ON runs(session_id);
