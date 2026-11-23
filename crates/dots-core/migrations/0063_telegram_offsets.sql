-- Where each Telegram bot's messages have been read up to (connected workflows spec section
-- 3), by the name of the Personal variable that holds its token: the `offset` the next
-- `getUpdates` asks from.
CREATE TABLE telegram_offsets (
    token_name  TEXT PRIMARY KEY,
    next_offset INTEGER NOT NULL
);
