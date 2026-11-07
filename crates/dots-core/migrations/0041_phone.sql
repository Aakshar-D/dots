-- Answer from your phone (phone view spec). The phones paired with this PC: only the SHA-256
-- hash of each phone's credential is kept, so the database alone cannot sign a phone in.
-- Removing a phone deletes its row, and its next request is refused.
CREATE TABLE phone_devices (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    token_hash   TEXT NOT NULL UNIQUE,
    created_at   TEXT NOT NULL,
    last_seen_at TEXT,
    last_ip      TEXT
);

-- What each phone did, newest last: the audit trail of the phone view. A row outlives the
-- phone that made it, so its name is copied in.
CREATE TABLE phone_actions (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    device_id   TEXT NOT NULL,
    device_name TEXT NOT NULL,
    action      TEXT NOT NULL,
    target      TEXT NOT NULL,
    summary     TEXT NOT NULL,
    at          TEXT NOT NULL
);

-- The listener's self-signed certificate. The private key is kept encrypted for the signed-in
-- Windows user (DPAPI); one row at most.
CREATE TABLE phone_tls (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    cert_der      BLOB NOT NULL,
    key_protected BLOB NOT NULL,
    created_at    TEXT NOT NULL
);

-- Where an approval was decided when it was not on this PC: "phone: <name>". NULL for a
-- decision made in the app.
ALTER TABLE approvals ADD COLUMN decided_via TEXT;
