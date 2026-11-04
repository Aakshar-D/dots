-- Snooze and Done on the Inbox (snooze and done spec section 4.2): one mark per Inbox item,
-- always in exactly one state. An item with no mark is shown normally.
CREATE TABLE inbox_marks (
    kind TEXT NOT NULL,
    item_id TEXT NOT NULL,
    snoozed_until TEXT,  -- RFC 3339 UTC; hidden until the sweep wakes it
    done_at TEXT,        -- marked done
    back_at TEXT,        -- woke from snooze: the "Back from snooze" tag
    marked_at TEXT NOT NULL,
    PRIMARY KEY (kind, item_id),
    CHECK ((snoozed_until IS NOT NULL) + (done_at IS NOT NULL) + (back_at IS NOT NULL) = 1)
);
CREATE INDEX inbox_marks_due ON inbox_marks (snoozed_until) WHERE snoozed_until IS NOT NULL;
