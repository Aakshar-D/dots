-- The Inbox asks for the newest run of each lineage on every runtime event.
CREATE INDEX runs_root ON runs(root_run_id, queued_at, id);
