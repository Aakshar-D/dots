-- Usage and limits (usage and limits spec section 7). The latest reading of each Claude limit
-- window the CLI reported, by the CLI's own window name; a run's API-equivalent cost; and
-- whether the person released a held run with Run anyway.
CREATE TABLE claude_limits (
    window TEXT PRIMARY KEY,
    utilization REAL NOT NULL,
    resets_at TEXT NOT NULL,
    status TEXT NOT NULL,
    read_at TEXT NOT NULL
);

ALTER TABLE runs ADD COLUMN cost_usd REAL;
ALTER TABLE runs ADD COLUMN hold_released INTEGER NOT NULL DEFAULT 0;
CREATE INDEX IF NOT EXISTS runs_started ON runs (started_at);
