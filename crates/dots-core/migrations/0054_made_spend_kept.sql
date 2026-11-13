-- What deleted dots spent, kept so it still counts toward their makers' caps (dots-make-dots
-- design section 6, Spend survives a delete). A dot's spend is summed from its runs, and
-- deleting a dot deletes its runs; so the transaction that deletes a dot with a maker first
-- writes one row here per run of this month that spent anything, for each maker up the chain as
-- it is then (`dot_id`), and once more with `dot_id` NULL, which is the copy that counts toward
-- the caps for every dot together. The runs of the living dots below the deleted one are kept
-- for the makers above it in the same way, since those dots stop sharing with them from then
-- on. `started_at` is the run's own, so the day and the month are bounded as they are for runs;
-- `tokens` is in + out. A maker's rows go with the maker; rows older than this month are
-- removed at the next delete of a dot.
CREATE TABLE made_spend_kept (
    id          INTEGER PRIMARY KEY,
    dot_id      TEXT REFERENCES dots(id) ON DELETE CASCADE,
    started_at  TEXT NOT NULL,
    cost_usd    REAL NOT NULL,
    tokens      INTEGER NOT NULL
);
CREATE INDEX made_spend_kept_dot ON made_spend_kept(dot_id, started_at);
