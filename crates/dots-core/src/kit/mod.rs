//! Skills and plugins on disk (MCP/skills spec sections 2 and 7): reading a `SKILL.md`, listing
//! the skills under a folder, reading a plugin folder, the plugins Claude Code has installed,
//! and the per-run plugin folders the Claude engine passes with `--plugin-dir`. It reads and
//! copies folders only; the Library, the runs and the engines call it.

mod frontmatter;
mod installed;
mod plugin;
mod run_dirs;
mod skill;

pub use frontmatter::{parse_frontmatter, remove_field, Frontmatter};
pub use installed::{
    find_installed, installed_plugins, plugins_root, read_installed, InstalledPlugin,
};
pub use plugin::{read_plugin, PluginInfo, PluginServer};
pub use run_dirs::{prepare_run_kit, KitPlugin, KitSkill, LeftOut, RunKit, MAX_COPY_BYTES};
pub use skill::{read_skill, scan_skills, SkillInfo, LISTING_MAX_CHARS};
