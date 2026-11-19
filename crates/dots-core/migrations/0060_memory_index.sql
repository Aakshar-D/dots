-- Ranked recall (spec 2026-11-19-memory-design.md, section 7): a full-text index over every
-- memory, a dot's (rows of `memories`) and a project's (files in the project's folder).
--
-- Both tables are a cache. `memory_docs` has one row for each memory the index knows: whose
-- it is (`scope` is `sol` or `project`; `owner` is the dot's id or the project's key), which
-- one (`memory_id`), a `stamp` of the text and files it was indexed with, and when `recall`
-- last answered with it. `memory_fts` holds the text and the file paths under the same row
-- id. No trigger writes here: before a search the store compares the rows with the memories
-- as they are now and brings them up under one write lock (`store/memory_index.rs`), as
-- `search_fts` is kept (migration 0037). Emptying both loses only the recall times.
CREATE TABLE memory_docs (
    doc_id           INTEGER PRIMARY KEY,
    scope            TEXT NOT NULL,
    owner            TEXT NOT NULL,
    memory_id        TEXT NOT NULL,
    stamp            TEXT NOT NULL,
    last_recalled_at TEXT,
    UNIQUE (scope, owner, memory_id)
);

CREATE VIRTUAL TABLE memory_fts USING fts5(
    text,
    files,
    tokenize = 'unicode61 remove_diacritics 2',
    prefix = '2 3'
);
-- A removed memory's words leave the index too.
INSERT INTO memory_fts (memory_fts, rank) VALUES ('secure-delete', 1);
