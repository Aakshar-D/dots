-- What the person narrowed on a made dot since its maker last set it (dots-make-dots design
-- section 9): a JSON list of settings (`making::reach::Narrowed`) their own edit took away or
-- lowered, such as the browser turned off or a variable removed. A maker's change that gives one
-- back asks the person. An entry goes once the dot has the setting again, by the person's edit
-- or with their say-so. NULL for a dot nothing of which was narrowed, and for a dot the person
-- made.
ALTER TABLE dots ADD COLUMN made_narrowed TEXT;
