-- Search (search spec 2026-11-06-search-design.md): one full-text index over what the person
-- and the dots said, memory, goals, topic titles and the names of files added to a chat. Each
-- `search_docs` row copies the text of one row of another table, as that table stores it (so
-- already scrubbed of secret values), and says what it is (`kind`), which row (`ref_id`, and
-- `seq` for a run's events), its dot and run when it has them, and when it was written.
-- Triggers on those tables keep the copies in step: a change replaces the copy, a delete (a
-- deleted dot's rows included) removes it.
--
-- `search_fts` indexes `body`, reading it from `search_docs`; secure-delete makes a removed
-- copy's words leave the index too. No trigger writes to it: a statement whose triggers reach
-- a virtual table reads the table's settings before it takes the write lock, which fails at
-- once with "database is locked" when another connection wrote meanwhile. The copies' changes
-- wait in `search_queue` instead (a new copy by its id, a removed one with its text), and the
-- store applies them under a write lock taken first (`store/search.rs`).
CREATE TABLE search_docs (
    doc_id INTEGER PRIMARY KEY,
    kind   TEXT NOT NULL,
    ref_id TEXT NOT NULL,
    seq    INTEGER NOT NULL DEFAULT 0,
    dot_id TEXT,
    run_id TEXT,
    at     TEXT NOT NULL,
    body   TEXT NOT NULL,
    UNIQUE (kind, ref_id, seq)
);
CREATE INDEX search_docs_run ON search_docs(run_id);

CREATE VIRTUAL TABLE search_fts USING fts5(
    body,
    content = 'search_docs',
    content_rowid = 'doc_id',
    tokenize = 'unicode61 remove_diacritics 2',
    prefix = '2 3'
);
INSERT INTO search_fts (search_fts, rank) VALUES ('secure-delete', 1);

CREATE TABLE search_queue (
    seq    INTEGER PRIMARY KEY,
    doc_id INTEGER NOT NULL,
    op     TEXT NOT NULL CHECK (op IN ('insert', 'delete')),
    body   TEXT
);
CREATE INDEX search_queue_doc ON search_queue(doc_id, op);

-- What the person said (`chat`): a chat run's message, or the answer it carries.
CREATE TRIGGER search_runs_ai AFTER INSERT ON runs WHEN NEW.trigger_kind = 'chat' BEGIN
    INSERT INTO search_docs (kind, ref_id, dot_id, run_id, at, body)
    SELECT 'chat', NEW.id, NEW.dot_id, NEW.id, NEW.queued_at, said FROM (SELECT
        CASE WHEN NOT json_valid(NEW.payload) THEN NULL
             WHEN json_type(NEW.payload, '$.message') = 'text'
                THEN json_extract(NEW.payload, '$.message')
             WHEN json_type(NEW.payload, '$.answer') = 'text'
                THEN json_extract(NEW.payload, '$.answer')
        END AS said)
    WHERE trim(said) <> '';
END;
CREATE TRIGGER search_runs_au_payload AFTER UPDATE OF payload ON runs BEGIN
    DELETE FROM search_docs WHERE kind = 'chat' AND ref_id = OLD.id;
    INSERT INTO search_docs (kind, ref_id, dot_id, run_id, at, body)
    SELECT 'chat', NEW.id, NEW.dot_id, NEW.id, NEW.queued_at, said FROM (SELECT
        CASE WHEN NOT json_valid(NEW.payload) THEN NULL
             WHEN json_type(NEW.payload, '$.message') = 'text'
                THEN json_extract(NEW.payload, '$.message')
             WHEN json_type(NEW.payload, '$.answer') = 'text'
                THEN json_extract(NEW.payload, '$.answer')
        END AS said)
    WHERE NEW.trigger_kind = 'chat' AND trim(said) <> '';
END;
-- What a dot said (`reply`): a run's result (`seq` 0) ...
CREATE TRIGGER search_runs_au_summary AFTER UPDATE OF summary ON runs BEGIN
    DELETE FROM search_docs WHERE kind = 'reply' AND ref_id = OLD.id AND seq = 0;
    INSERT INTO search_docs (kind, ref_id, seq, dot_id, run_id, at, body)
    SELECT 'reply', NEW.id, 0, NEW.dot_id, NEW.id, COALESCE(NEW.ended_at, NEW.queued_at),
        NEW.summary
    WHERE trim(NEW.summary) <> '';
END;
CREATE TRIGGER search_runs_ad AFTER DELETE ON runs BEGIN
    DELETE FROM search_docs WHERE run_id = OLD.id AND kind IN ('chat', 'reply');
END;
-- ... and the text it wrote along the way (`seq` is the event's).
CREATE TRIGGER search_events_ai AFTER INSERT ON run_events
WHEN NEW.kind = 'assistant_text' BEGIN
    INSERT INTO search_docs (kind, ref_id, seq, dot_id, run_id, at, body)
    SELECT 'reply', NEW.run_id, NEW.seq, r.dot_id, NEW.run_id, NEW.ts, said FROM (SELECT
        CASE WHEN NOT json_valid(NEW.data) THEN NULL
             WHEN json_type(NEW.data, '$.text') = 'text' THEN json_extract(NEW.data, '$.text')
        END AS said)
    JOIN runs r ON r.id = NEW.run_id
    WHERE trim(said) <> '';
END;
CREATE TRIGGER search_events_ad AFTER DELETE ON run_events
WHEN OLD.kind = 'assistant_text' BEGIN
    DELETE FROM search_docs WHERE kind = 'reply' AND ref_id = OLD.run_id AND seq = OLD.seq;
END;

-- A note or question a dot posted (`note`).
CREATE TRIGGER search_messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO search_docs (kind, ref_id, dot_id, run_id, at, body)
    SELECT 'note', NEW.id, NEW.dot_id, NEW.run_id, NEW.created_at, NEW.text
    WHERE trim(NEW.text) <> '';
END;
CREATE TRIGGER search_messages_au AFTER UPDATE OF text ON messages BEGIN
    DELETE FROM search_docs WHERE kind = 'note' AND ref_id = OLD.id;
    INSERT INTO search_docs (kind, ref_id, dot_id, run_id, at, body)
    SELECT 'note', NEW.id, NEW.dot_id, NEW.run_id, NEW.created_at, NEW.text
    WHERE trim(NEW.text) <> '';
END;
CREATE TRIGGER search_messages_ad AFTER DELETE ON messages BEGIN
    DELETE FROM search_docs WHERE kind = 'note' AND ref_id = OLD.id;
END;

-- A memory (`memory`).
CREATE TRIGGER search_memories_ai AFTER INSERT ON memories BEGIN
    INSERT INTO search_docs (kind, ref_id, dot_id, at, body)
    SELECT 'memory', NEW.id, NEW.dot_id, NEW.updated_at, NEW.text WHERE trim(NEW.text) <> '';
END;
CREATE TRIGGER search_memories_au AFTER UPDATE OF text ON memories BEGIN
    DELETE FROM search_docs WHERE kind = 'memory' AND ref_id = OLD.id;
    INSERT INTO search_docs (kind, ref_id, dot_id, at, body)
    SELECT 'memory', NEW.id, NEW.dot_id, NEW.updated_at, NEW.text WHERE trim(NEW.text) <> '';
END;
CREATE TRIGGER search_memories_ad AFTER DELETE ON memories BEGIN
    DELETE FROM search_docs WHERE kind = 'memory' AND ref_id = OLD.id;
END;

-- A goal (`intent`). Its dot and its account are read from the goal when searching, since
-- they can change.
CREATE TRIGGER search_intents_ai AFTER INSERT ON intents BEGIN
    INSERT INTO search_docs (kind, ref_id, at, body)
    SELECT 'intent', NEW.id, NEW.created_at, NEW.text WHERE trim(NEW.text) <> '';
END;
CREATE TRIGGER search_intents_au AFTER UPDATE OF text ON intents BEGIN
    DELETE FROM search_docs WHERE kind = 'intent' AND ref_id = OLD.id;
    INSERT INTO search_docs (kind, ref_id, at, body)
    SELECT 'intent', NEW.id, NEW.created_at, NEW.text WHERE trim(NEW.text) <> '';
END;
CREATE TRIGGER search_intents_ad AFTER DELETE ON intents BEGIN
    DELETE FROM search_docs WHERE kind = 'intent' AND ref_id = OLD.id;
END;

-- A topic's title (`topic`); `ref_id` and `run_id` are its root run.
CREATE TRIGGER search_topics_ai AFTER INSERT ON topics BEGIN
    INSERT INTO search_docs (kind, ref_id, dot_id, run_id, at, body)
    SELECT 'topic', NEW.root_run_id, NEW.dot_id, NEW.root_run_id,
        COALESCE((SELECT queued_at FROM runs WHERE id = NEW.root_run_id), ''), NEW.title
    WHERE trim(NEW.title) <> '';
END;
CREATE TRIGGER search_topics_au AFTER UPDATE OF title, root_run_id ON topics BEGIN
    DELETE FROM search_docs WHERE kind = 'topic' AND ref_id = OLD.root_run_id;
    INSERT INTO search_docs (kind, ref_id, dot_id, run_id, at, body)
    SELECT 'topic', NEW.root_run_id, NEW.dot_id, NEW.root_run_id,
        COALESCE((SELECT queued_at FROM runs WHERE id = NEW.root_run_id), ''), NEW.title
    WHERE trim(NEW.title) <> '';
END;
CREATE TRIGGER search_topics_ad AFTER DELETE ON topics BEGIN
    DELETE FROM search_docs WHERE kind = 'topic' AND ref_id = OLD.root_run_id;
END;

-- The name of a file added to a chat (`file`), once a message took it: a staged file is still
-- the person's draft. The name in the topic's folder when it has one.
CREATE TRIGGER search_files_ai AFTER INSERT ON chat_attachments
WHEN NEW.run_id IS NOT NULL BEGIN
    INSERT INTO search_docs (kind, ref_id, dot_id, run_id, at, body)
    VALUES ('file', NEW.id, NEW.dot_id, NEW.run_id, NEW.created_at,
        COALESCE(NEW.name, NEW.file_name));
END;
CREATE TRIGGER search_files_au AFTER UPDATE OF run_id, name, file_name ON chat_attachments
BEGIN
    DELETE FROM search_docs WHERE kind = 'file' AND ref_id = OLD.id;
    INSERT INTO search_docs (kind, ref_id, dot_id, run_id, at, body)
    SELECT 'file', NEW.id, NEW.dot_id, NEW.run_id, NEW.created_at,
        COALESCE(NEW.name, NEW.file_name)
    WHERE NEW.run_id IS NOT NULL;
END;
CREATE TRIGGER search_files_ad AFTER DELETE ON chat_attachments BEGIN
    DELETE FROM search_docs WHERE kind = 'file' AND ref_id = OLD.id;
END;

-- What was stored before this migration.
INSERT INTO search_docs (kind, ref_id, dot_id, run_id, at, body)
SELECT 'chat', id, dot_id, id, queued_at, said FROM (SELECT id, dot_id, queued_at,
    CASE WHEN NOT json_valid(payload) THEN NULL
         WHEN json_type(payload, '$.message') = 'text' THEN json_extract(payload, '$.message')
         WHEN json_type(payload, '$.answer') = 'text' THEN json_extract(payload, '$.answer')
    END AS said
    FROM runs WHERE trigger_kind = 'chat')
WHERE trim(said) <> '';
INSERT INTO search_docs (kind, ref_id, seq, dot_id, run_id, at, body)
SELECT 'reply', id, 0, dot_id, id, COALESCE(ended_at, queued_at), summary FROM runs
WHERE trim(summary) <> '';
INSERT INTO search_docs (kind, ref_id, seq, dot_id, run_id, at, body)
SELECT 'reply', e.run_id, e.seq, r.dot_id, e.run_id, e.ts, said FROM (SELECT run_id, seq, ts,
    CASE WHEN NOT json_valid(data) THEN NULL
         WHEN json_type(data, '$.text') = 'text' THEN json_extract(data, '$.text')
    END AS said
    FROM run_events WHERE kind = 'assistant_text') e
JOIN runs r ON r.id = e.run_id
WHERE trim(said) <> '';
INSERT INTO search_docs (kind, ref_id, dot_id, run_id, at, body)
SELECT 'note', id, dot_id, run_id, created_at, text FROM messages WHERE trim(text) <> '';
INSERT INTO search_docs (kind, ref_id, dot_id, at, body)
SELECT 'memory', id, dot_id, updated_at, text FROM memories WHERE trim(text) <> '';
INSERT INTO search_docs (kind, ref_id, at, body)
SELECT 'intent', id, created_at, text FROM intents WHERE trim(text) <> '';
INSERT INTO search_docs (kind, ref_id, dot_id, run_id, at, body)
SELECT 'topic', t.root_run_id, t.dot_id, t.root_run_id, COALESCE(r.queued_at, ''), t.title
FROM topics t LEFT JOIN runs r ON r.id = t.root_run_id
WHERE trim(t.title) <> '';
INSERT INTO search_docs (kind, ref_id, dot_id, run_id, at, body)
SELECT 'file', id, dot_id, run_id, created_at, COALESCE(name, file_name)
FROM chat_attachments WHERE run_id IS NOT NULL;

-- The index of what was stored before; from now on the queue brings each change.
INSERT INTO search_fts (search_fts) VALUES ('rebuild');

CREATE TRIGGER search_docs_ai AFTER INSERT ON search_docs BEGIN
    INSERT INTO search_queue (doc_id, op) VALUES (NEW.doc_id, 'insert');
END;
-- A copy the index never took leaves the queue; one it took is removed from it by its text.
CREATE TRIGGER search_docs_ad AFTER DELETE ON search_docs BEGIN
    INSERT INTO search_queue (doc_id, op, body)
    SELECT OLD.doc_id, 'delete', OLD.body WHERE NOT EXISTS
        (SELECT 1 FROM search_queue WHERE doc_id = OLD.doc_id AND op = 'insert');
    DELETE FROM search_queue WHERE doc_id = OLD.doc_id AND op = 'insert';
END;
-- A copy is never changed in place: a change removes it and adds another.
CREATE TRIGGER search_docs_bu BEFORE UPDATE ON search_docs BEGIN
    SELECT RAISE(ABORT, 'search_docs rows are replaced, not changed');
END;
