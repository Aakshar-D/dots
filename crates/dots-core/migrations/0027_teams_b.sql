ALTER TABLE workflow_step_runs ADD COLUMN outcome TEXT;
ALTER TABLE workflow_step_runs ADD COLUMN notice TEXT;
ALTER TABLE workflow_step_runs ADD COLUMN send_back TEXT;
-- The members a delegate step's run started (teams spec section 5.3): one row per member
-- thread, keyed by its root run. The delegating run is continued with the results of those
-- not yet reported.
CREATE TABLE workflow_step_members (
    run_id TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    step_run_id TEXT NOT NULL REFERENCES workflow_step_runs(id) ON DELETE CASCADE,
    dot_id TEXT NOT NULL,
    task TEXT NOT NULL,
    reported INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
CREATE INDEX workflow_step_members_step ON workflow_step_members(step_run_id, created_at);
-- The last send-back of a workflow run (teams spec sections 5.4 and 9), JSON
-- `{"from", "to", "note", "by", "held"}`: the one performed, or the one held at the cap.
ALTER TABLE workflow_runs ADD COLUMN send_back TEXT;
-- What a deploy step did (teams spec section 5.5), JSON `{"branches": [{"branch", "commit",
-- "pushed", "pr_url", "auto_merge"}], "ready"}`.
ALTER TABLE workflow_step_runs ADD COLUMN deploy_result TEXT;
