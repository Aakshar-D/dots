//! Browser use for dots (browser spec `docs/superpowers/specs/2026-10-21-browser-design.md`):
//! Microsoft Edge in a visible window with the dot's own profile, driven by Microsoft's
//! Playwright MCP server, behind Dots' gate, which judges each call by the address of the page
//! it acts on (`sites`) and the labels in Playwright's own snapshots.

pub mod sites;
