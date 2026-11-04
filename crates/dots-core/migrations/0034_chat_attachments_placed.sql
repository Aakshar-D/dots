-- Attachments in chat (attachments spec section 4.4): when Dots first put the file in its topic's
-- folder. A file placed before is taken as it is there when a run of its message starts again
-- (Retry, a restart), so an edit to it is kept; one never placed must find its name free.
ALTER TABLE chat_attachments ADD COLUMN placed_at TEXT;
