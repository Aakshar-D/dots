ALTER TABLE runs ADD COLUMN delivered_at TEXT;    -- engine started on it (RFC 3339 UTC)
ALTER TABLE runs ADD COLUMN undelivered_at TEXT;  -- marked not delivered (ended undelivered, or 30 min waiting)
ALTER TABLE runs ADD COLUMN undelivered_reason TEXT;
ALTER TABLE runs ADD COLUMN dismissed_at TEXT;    -- the person retried or discarded it
UPDATE runs SET delivered_at = started_at WHERE started_at IS NOT NULL;
UPDATE runs SET dismissed_at = ended_at WHERE ended_at IS NOT NULL AND delivered_at IS NULL;
