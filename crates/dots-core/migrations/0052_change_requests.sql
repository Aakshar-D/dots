-- Change requests (dots-make-dots design section 9): a change a maker asked for a dot it made
-- that gives the dot more than the maker has waits in `make_requests` with `kind` `change` and
-- `dot_id` the made dot. `ask` is the change as the maker asked it, JSON (`ChangeDot`): when the
-- person answers, it is put on the dot as the dot is then, so what they edited since stays.
-- NULL for a make.
ALTER TABLE make_requests ADD COLUMN ask TEXT;
