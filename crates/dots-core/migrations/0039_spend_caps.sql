-- Spending caps (spend caps design). A dot's daily caps: its estimated API cost in USD and its
-- tokens (in + out) since local midnight; NULL is no cap. `cap_released` marks a run the person
-- let start once past a cap that had been reached (**Run once anyway**).
ALTER TABLE dots ADD COLUMN spend_daily_usd REAL;
ALTER TABLE dots ADD COLUMN spend_daily_tokens INTEGER;
ALTER TABLE runs ADD COLUMN cap_released INTEGER NOT NULL DEFAULT 0;
