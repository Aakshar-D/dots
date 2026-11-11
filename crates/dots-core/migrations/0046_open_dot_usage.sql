-- Open Dot, the voice assistant: what each of its requests to the model used. It counts toward
-- the caps for every dot together, never toward a dot's own, and shows on the Usage page.
CREATE TABLE open_dot_usage (
    id TEXT PRIMARY KEY,
    at TEXT NOT NULL,
    model TEXT NOT NULL,
    tokens_in INTEGER NOT NULL,
    tokens_out INTEGER NOT NULL,
    cache_read INTEGER NOT NULL,
    cost_usd REAL
);
CREATE INDEX open_dot_usage_at ON open_dot_usage (at);
