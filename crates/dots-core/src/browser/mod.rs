//! Browser use for dots (browser spec `docs/superpowers/specs/2026-10-21-browser-design.md`):
//! Microsoft Edge in a visible window with the dot's own profile, driven by Microsoft's
//! Playwright MCP server, behind Dots' gate, which judges each call by the address of the page
//! it acts on (`sites`) and the labels in Playwright's own snapshots.

pub mod gate;
pub mod host;
pub mod sites;
pub mod snapshot;

/// The built-in server's name (browser spec section 2.2): its tools are `mcp__browser__<tool>`
/// on both engines.
pub const SERVER: &str = "browser";
/// The prefix of its tools' names.
pub const PREFIX: &str = "mcp__browser__";

/// Whether `qualified` names a tool of the built-in browser.
pub fn is_browser_tool(qualified: &str) -> bool {
    qualified.len() > PREFIX.len() && qualified.starts_with(PREFIX)
}
