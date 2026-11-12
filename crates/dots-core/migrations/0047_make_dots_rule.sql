-- Every preset allows `MakeDots`, the one policy name of the tools for making dots
-- (dots-make-dots design section 5): the dot's "Can make dots" switch is the gate, and the name
-- lets the person add an ask or deny rule. The dots whose policy gets the rule, Custom too as
-- 0018 did for `Instructions`, when the store opens (`store/upgrade.rs`, `finish_0047`). The
-- rows wait here, and the table is dropped once they are done.
CREATE TABLE upgrade_0047 (
    dot_id TEXT PRIMARY KEY REFERENCES dots(id) ON DELETE CASCADE
);
INSERT INTO upgrade_0047 (dot_id) SELECT id FROM dots;
