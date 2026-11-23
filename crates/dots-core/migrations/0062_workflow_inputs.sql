-- What a workflow run was started with besides its task (general workflows spec section 3):
-- a JSON object of each input's name to its text, the defaults filled in.
ALTER TABLE workflow_runs ADD COLUMN inputs TEXT NOT NULL DEFAULT '{}';
