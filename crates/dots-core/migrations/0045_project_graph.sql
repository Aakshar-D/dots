-- "Use project graph" (project graph spec section 6): the dot's runs get the `graph` tool and
-- the project map. Existing dots have it on. The graphs themselves live in their own files under
-- the data folder, not in this database.
ALTER TABLE dots ADD COLUMN use_graph INTEGER NOT NULL DEFAULT 1;
