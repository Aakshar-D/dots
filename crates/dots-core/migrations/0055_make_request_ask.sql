-- Make requests keep what the maker asked (dots-make-dots design section 7.2): `make_ask` is
-- the `make_dot` call as the model sent it, JSON, for a request of the kind `make`, so its card
-- and the person's answer put it against the maker as it is then. NULL for a change, whose
-- `ask` holds the change, and for a request stored before this column.
ALTER TABLE make_requests ADD COLUMN make_ask TEXT;

-- The cards of a maker's chat topic read its requests by topic.
CREATE INDEX make_requests_topic ON make_requests(root_run_id);

-- A delete reads the topics whose log names the dot (`Store::making_topics_of`).
CREATE INDEX making_events_dot ON making_events(dot_id);
