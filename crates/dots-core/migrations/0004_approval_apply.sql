-- How a late decision on an approval is carried out (Approvals v2 spec, section 6).
-- `self_apply`: Dots, not the model, performs the call when it is approved after the engine
-- stopped waiting. `apply_state`: where that call stands; NULL for every other approval.
ALTER TABLE approvals ADD COLUMN self_apply INTEGER NOT NULL DEFAULT 0;
ALTER TABLE approvals ADD COLUMN apply_state TEXT;
ALTER TABLE approvals ADD COLUMN applied_at TEXT;
