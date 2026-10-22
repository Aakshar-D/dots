-- Browser use (spec 2026-10-21-browser-design.md, section 6): a dot with `browser` on gets
-- Microsoft Edge with its own profile, driven through Playwright MCP. Every stored dot is off.
ALTER TABLE dots ADD COLUMN browser INTEGER NOT NULL DEFAULT 0;
