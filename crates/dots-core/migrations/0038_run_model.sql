-- The model each run was started with, and Auto's pick (auto model spec section 5): the size
-- (`small`, `standard`, `large`), why in a few words, and the model it tried first when the
-- CLI could not use that one. All NULL for runs from before, and the last three for a dot with
-- a model of its own.
ALTER TABLE runs ADD COLUMN model TEXT;
ALTER TABLE runs ADD COLUMN model_tier TEXT;
ALTER TABLE runs ADD COLUMN model_reason TEXT;
ALTER TABLE runs ADD COLUMN model_fallback_from TEXT;
