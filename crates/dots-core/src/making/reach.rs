//! The reach check (dots-make-dots design section 6): given a maker as it is now and what it
//! asks for a new dot, or as a change to a dot it made, the whole dot asked for, the parts of
//! it beyond the maker's reach (the extras), and the same dot without them.
//!
//! It compares with the maker's own settings only, and writes nothing: no store, no folder. It
//! reads the file system to resolve real paths. Whether the Library has an item and the vault
//! a variable, the limits (section 10), and that the maker made the dot it changes are the
//! caller's checks.

use std::path::{Component, Path, PathBuf, Prefix};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::library::{LibraryKind, Uses};
use crate::model::{Dot, DotSpec, Making, WorkspaceMode};
use crate::policy::{Action, Policy, Preset};
use crate::schedule::Schedule;
use crate::vault::{Consumer, Grants};
use crate::{Error, Result};

/// What `permissions` says for an exact copy of the maker's whole policy.
pub const MAKERS_PERMISSIONS: &str = "maker";
/// What `schedule` says for a dot without a schedule.
pub const NO_SCHEDULE: &str = "none";

/// What a maker asks for (design sections 7.1 and 9). Every field is optional: a new dot needs
/// `name` and `instructions`, and a change changes only the fields it names. In JSON a field
/// that is missing or `null` is not named, and a field this struct does not have is refused.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Wanted {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// A model of the maker's engine; the maker's when a new dot names none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The folder the dot works in: absolute, or relative to the maker's folder. Blank is as
    /// not named: a new dot gets a new folder of its own, a changed dot keeps its folder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<WantedPermissions>,
    /// The whole set of Library items the dot is to use; none when a new dot names none. A
    /// group `Uses` does not have is refused, not dropped.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "known_uses"
    )]
    pub uses: Option<Uses>,
    /// The whole set of variables the dot is to be granted, by consumer (`shell` or
    /// `mcp:<server>`); none when a new dot names none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variables: Option<Grants>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_user_settings: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// `Some(None)` is no schedule: it clears the schedule of a changed dot. In JSON a
    /// schedule is its object and no schedule is the word `"none"`; `null` is not named, since
    /// a model writes `null` for a field it leaves out.
    #[serde(skip_serializing_if = "Option::is_none", with = "schedule_or_none")]
    pub schedule: Option<Option<Schedule>>,
}

/// The groups of `uses`, as `Uses` reads them.
const USES_GROUPS: [&str; 5] = ["mcp_servers", "skills", "plugins", "context", "apps"];

/// `uses` as `Uses` reads it, but a group it does not have (`"skill"`) is refused by name, so
/// the maker learns that nothing of it was asked. `Uses` itself stays lenient: stored dots and
/// the other callers read it.
fn known_uses<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Option<Uses>, D::Error> {
    use serde::de::Error as _;
    let value = serde_json::Value::deserialize(d)?;
    if let Some(groups) = value.as_object() {
        if let Some(unknown) = groups
            .keys()
            .find(|key| !USES_GROUPS.contains(&key.as_str()))
        {
            return Err(D::Error::custom(format!(
                "uses has no group {unknown:?}; its groups are {}",
                USES_GROUPS.join(", ")
            )));
        }
    }
    Option::<Uses>::deserialize(value).map_err(D::Error::custom)
}

mod schedule_or_none {
    use super::*;

    pub fn serialize<S: Serializer>(
        value: &Option<Option<Schedule>>,
        s: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        match value {
            None => s.serialize_none(),
            Some(None) => s.serialize_str(NO_SCHEDULE),
            Some(Some(schedule)) => schedule.serialize(s),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> std::result::Result<Option<Option<Schedule>>, D::Error> {
        use serde::de::Error as _;
        match serde_json::Value::deserialize(d)? {
            serde_json::Value::Null => Ok(None),
            serde_json::Value::String(word) if word == NO_SCHEDULE => Ok(Some(None)),
            serde_json::Value::String(word) => Err(D::Error::custom(format!(
                "schedule is a schedule or \"{NO_SCHEDULE}\", not {word:?}"
            ))),
            value => serde_json::from_value(value)
                .map(|schedule| Some(Some(schedule)))
                .map_err(D::Error::custom),
        }
    }
}

/// The permissions a maker asks for (design section 6, Permissions). In JSON, `"maker"` or
/// the preset's name: `"read-only"`, `"sandboxed"` or `"trusted"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WantedPermissions {
    /// An exact copy of the maker's whole policy, as it is at that moment.
    Maker,
    /// A plain preset with no added rules. `Preset::Custom` is no plain preset and is refused.
    Preset(Preset),
}

/// What is said of a `permissions` that is neither the maker's nor a plain preset.
const PERMISSIONS_RULE: &str =
    "permissions is \"maker\", \"read-only\", \"sandboxed\" or \"trusted\"";

impl Serialize for WantedPermissions {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(match self {
            Self::Maker => MAKERS_PERMISSIONS,
            Self::Preset(preset) => preset.as_str(),
        })
    }
}

impl<'de> Deserialize<'de> for WantedPermissions {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        use serde::de::Error as _;
        let word = String::deserialize(d)?;
        if word == MAKERS_PERMISSIONS {
            return Ok(Self::Maker);
        }
        match Preset::parse(&word) {
            Ok(preset) if preset != Preset::Custom => Ok(Self::Preset(preset)),
            _ => Err(D::Error::custom(format!(
                "{PERMISSIONS_RULE}, not {word:?}"
            ))),
        }
    }
}

/// One thing beyond the maker's reach (design section 6). It is stored as JSON with a make
/// request, and says itself in plain words on the ask card (section 7.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Extra {
    /// A Library item the maker does not use.
    Item {
        item_kind: LibraryKind,
        name: String,
    },
    /// A variable the maker is not granted for that consumer (`shell` or `mcp:<server>`).
    Variable { consumer: String, name: String },
    /// A plain preset stronger than the maker's (`makers`); when the maker's is custom, any
    /// but read-only, and read-only too unless the maker's rules allow all that read-only
    /// allows.
    Preset { preset: Preset, makers: Preset },
    /// A folder outside the maker's folder, or, with `below_maker`, a folder below the folder
    /// of a maker that works in worktree copies. `path` is the real path.
    Folder {
        path: String,
        #[serde(default)]
        below_maker: bool,
    },
    /// A folder below the maker's that is, or is inside, git's or Claude's own folder (`.git`,
    /// `.claude`), whose settings and hooks run code. `path` is the real path.
    SettingsFolder { path: String },
    /// The browser, which the maker does not have.
    Browser,
    /// The person's own Claude settings, which the maker does not use.
    UserSettings,
    /// A turn limit above the maker's.
    MaxTurns { wanted: u32, makers: u32 },
    /// A time limit above the maker's.
    Timeout { wanted_secs: u64, makers_secs: u64 },
}

/// A preset as the person reads it.
pub(super) fn preset_label(preset: Preset) -> &'static str {
    match preset {
        Preset::ReadOnly => "Read-only",
        Preset::Sandboxed => "Sandboxed",
        Preset::Trusted => "Trusted",
        Preset::Custom => "Custom",
    }
}

/// A time limit as the person reads it: "30 min", "2 h", "1 h 30 min", "45 s".
pub(super) fn time_label(secs: u64) -> String {
    let (hours, minutes, seconds) = (secs / 3600, secs % 3600 / 60, secs % 60);
    let mut parts = Vec::new();
    if hours > 0 {
        parts.push(format!("{hours} h"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes} min"));
    }
    if seconds > 0 || parts.is_empty() {
        parts.push(format!("{seconds} s"));
    }
    parts.join(" ")
}

impl Extra {
    /// The extra as one line of the ask card (design section 7.2). `maker` is the maker's name.
    pub fn line(&self, maker: &str) -> String {
        match self {
            Self::Item { item_kind, name } => format!("The {} '{name}'", item_kind.label()),
            Self::Variable { consumer, name } => match Consumer::parse(consumer) {
                Ok(Consumer::Mcp(server)) => {
                    format!("The variable {name} for the server '{server}'")
                }
                Ok(Consumer::Shell) => format!("The variable {name} for the shell"),
                Err(_) => format!("The variable {name} for {consumer}"),
            },
            Self::Preset { preset, makers } => match makers {
                Preset::Custom => format!(
                    "The preset {} ({maker} has its own rules)",
                    preset_label(*preset)
                ),
                makers => format!(
                    "The preset {} ({maker} has {})",
                    preset_label(*preset),
                    preset_label(*makers)
                ),
            },
            Self::Folder {
                path,
                below_maker: false,
            } => format!("The folder {path}, outside {maker}'s folder"),
            Self::Folder {
                path,
                below_maker: true,
            } => format!(
                "The folder {path}, inside {maker}'s folder ({maker} itself works in copies of it)"
            ),
            Self::SettingsFolder { path } => format!(
                "The folder {path}, inside {maker}'s folder, where git or Claude keep their own \
                 settings"
            ),
            Self::Browser => "The browser".to_string(),
            Self::UserSettings => "Your Claude settings".to_string(),
            Self::MaxTurns { wanted, makers } => {
                format!("A turn limit of {wanted} ({maker} has {makers})")
            }
            Self::Timeout {
                wanted_secs,
                makers_secs,
            } => format!(
                "A time limit of {} ({maker} has {})",
                time_label(*wanted_secs),
                time_label(*makers_secs)
            ),
        }
    }
}

/// What the reach check found.
#[derive(Debug, Clone, PartialEq)]
pub struct Reach {
    /// The dot as asked, ready to save if the person allows the extras. A blank `workdir` is a
    /// new dot that gets a new folder of its own when it is created (`DotAdmin::create`).
    pub spec: DotSpec,
    /// The same with the extras removed (design section 7.2, "Make it without these"): the
    /// items and variables dropped, the browser and the user settings off, the permissions
    /// and the limits the maker's, the folder a new one of its own. For a change, a part that
    /// is an extra stays as the dot has it, except that a limit or a plain preset below the
    /// maker's is raised to the maker's.
    pub without: DotSpec,
    /// Empty when everything is within reach; then `spec == without`.
    pub extras: Vec<Extra>,
    /// The folder of `spec` when it lies below the maker's folder and does not exist yet: the
    /// caller creates it before saving `spec` (design section 6, folder rules). `without` needs
    /// it only when the folder is no extra.
    pub create_folder: Option<PathBuf>,
}

/// `path` without Windows' `\\?\` prefix, which `std::fs::canonicalize` adds and which git and
/// the `claude` CLI do not take. A UNC path keeps it.
fn plain(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => path,
    }
}

/// The real path of the absolute `path` (design section 6: case, `..`, links and junctions):
/// each part that exists is resolved as the file system has it, and the parts after the last
/// one that exists are kept as given, so a folder that is still to be made has a real path
/// too. A `..` leaves the real folder it follows.
pub fn real_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(_) => {
                out.push(part);
                if let Ok(real) = std::fs::canonicalize(&out) {
                    out = real;
                }
            }
            // A drive alone would resolve to the current folder on it.
            Component::Prefix(_) | Component::RootDir => out.push(part),
        }
    }
    plain(out)
}

/// A real path as it is compared: Windows paths ignore case, and a part that does not exist
/// yet keeps the case it was given in.
pub(super) fn folded(path: &Path) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    } else {
        path.to_path_buf()
    }
}

/// Where a named folder puts the dot (design section 6, folder rules).
struct Placed {
    /// Its real path.
    real: PathBuf,
    /// The dot's `workdir`: the maker's own as the maker has it, else the real path.
    workdir: String,
    mode: WorkspaceMode,
    /// Set when the folder is beyond the maker's reach.
    extra: Option<Extra>,
    /// Set when it lies below the maker's folder and does not exist yet.
    create: Option<PathBuf>,
}

/// What is said of a folder on another computer or a device.
const ELSEWHERE: &str = "That folder is on another computer or a device. Name a folder on this PC.";

/// Refuses, before anything reads the disk, a folder on another computer or a device (a
/// `\\server\share` path, `\\?\UNC\…`, `\\.\…`), whose mere lookup would connect to it, and a
/// drive without its root (`C:notes`), which means a different folder at each moment.
fn refuse_unplain(given: &str, maker: &str) -> Result<()> {
    let mut parts = Path::new(given).components();
    let Some(Component::Prefix(prefix)) = parts.next() else {
        return Ok(());
    };
    if !matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)) {
        return Err(Error::Invalid(ELSEWHERE.into()));
    }
    if !matches!(parts.next(), Some(Component::RootDir)) {
        return Err(Error::Invalid(format!(
            "Name the folder by its full path, such as {}\\notes, or by its path from \
             {maker}'s folder.",
            prefix.as_os_str().to_string_lossy()
        )));
    }
    Ok(())
}

/// Whether a part of `path` is a link or junction whose target cannot be found: the folder it
/// names would follow the link once a target appears, wherever that is.
fn dangling_link(path: &Path) -> bool {
    path.ancestors().any(|part| {
        std::fs::symlink_metadata(part).is_ok_and(|meta| meta.file_type().is_symlink())
            && std::fs::canonicalize(part).is_err()
    })
}

/// Whether `part` of a path names git's or Claude's own folder, `.git` or `.claude`, in any
/// case. Windows drops trailing dots and spaces, and reads `name::$INDEX_ALLOCATION` as the
/// folder `name`.
fn settings_part(part: &std::ffi::OsStr) -> bool {
    let text = part.to_string_lossy();
    let name = text
        .split(':')
        .next()
        .unwrap_or_default()
        .trim_end_matches(['.', ' ']);
    name.eq_ignore_ascii_case(".git") || name.eq_ignore_ascii_case(".claude")
}

/// Applies the folder rules to `given`, a folder that is not blank: absolute, or relative to
/// the maker's folder.
fn place(maker: &Dot, given: &str, worktrees_root: &Path) -> Result<Placed> {
    refuse_unplain(given, &maker.spec.name)?;
    let own = maker.spec.workdir.trim();
    // A maker whose folder is not a full path (".", "C:") has no folder a path can be inside:
    // every folder is beyond its reach, and one relative to it cannot be found.
    let anchored = Path::new(own).is_absolute();
    if !anchored && !Path::new(given).is_absolute() {
        return Err(Error::Invalid(format!(
            "Name the folder by its full path: {}'s own folder is not one.",
            maker.spec.name
        )));
    }
    let makers = real_path(Path::new(own));
    // The path as Windows reads it where the dot uses it (`GetFullPathNameW`, which reads no
    // disk): `.. ` is `..` and a trailing dot goes, which `real_path`'s long form keeps.
    let joined = std::path::absolute(makers.join(given))
        .map_err(|_| Error::Invalid(format!("{given} is not a folder path.")))?;
    if let Some(Component::Prefix(prefix)) = joined.components().next() {
        // A device name (`nul`, `com1`) is a device, not a folder.
        if matches!(prefix.kind(), Prefix::DeviceNS(_)) {
            return Err(Error::Invalid(ELSEWHERE.into()));
        }
    }
    let real = real_path(&joined);
    let (key, makers_key) = (folded(&real), folded(&makers));
    // A worktree copy goes when its run is discarded or merged, so no card can allow it.
    if !worktrees_root.as_os_str().is_empty() && key.starts_with(folded(&real_path(worktrees_root)))
    {
        return Err(Error::Invalid(format!(
            "That folder is a run's working copy, which does not last. Leave the folder blank \
             for a new one, or use {}'s own folder.",
            maker.spec.name
        )));
    }
    if anchored && key == makers_key {
        // The maker's mode too, so a made dot of a worktree maker works in worktree copies.
        return Ok(Placed {
            real,
            workdir: maker.spec.workdir.clone(),
            mode: maker.spec.workspace_mode,
            extra: None,
            create: None,
        });
    }
    // The nearest part of it that exists must be a folder.
    let mut nearest = real.as_path();
    while !nearest.exists() {
        match nearest.parent() {
            Some(parent) => nearest = parent,
            None => break,
        }
    }
    if nearest.is_file() {
        return Err(Error::Invalid(format!(
            "{} is a file, not a folder.",
            nearest.display()
        )));
    }
    let missing = !real.is_dir();
    let workdir = real.to_string_lossy().into_owned();
    // Inside by its path; a dangling link in it may lead anywhere, so it is not inside.
    let below = anchored && key.starts_with(&makers_key) && !dangling_link(&real);
    if below {
        // Git's and Claude's own folders hold settings and hooks that run code, which no
        // preset but Trusted lets a dot write: one that works there writes them by a relative
        // path, which the deny rules of `.git` and `.claude` do not see.
        let settings = key.strip_prefix(&makers_key).is_ok_and(|tail| {
            tail.components()
                .any(|part| settings_part(part.as_os_str()))
        });
        // A worktree maker only ever writes in worktree copies; a dot in a subfolder would
        // write straight into the person's checkout, which is more than the maker can do.
        let extra = if settings {
            Some(Extra::SettingsFolder {
                path: workdir.clone(),
            })
        } else {
            (maker.spec.workspace_mode == WorkspaceMode::Worktree).then(|| Extra::Folder {
                path: workdir.clone(),
                below_maker: true,
            })
        };
        return Ok(Placed {
            create: missing.then(|| real.clone()),
            real,
            workdir,
            mode: WorkspaceMode::Folder,
            extra,
        });
    }
    if missing {
        return Err(Error::Invalid(format!(
            "The folder {workdir} does not exist. Leave the folder blank for a new one, or name \
             one that exists."
        )));
    }
    Ok(Placed {
        real,
        extra: Some(Extra::Folder {
            path: workdir.clone(),
            below_maker: false,
        }),
        workdir,
        mode: WorkspaceMode::Folder,
        create: None,
    })
}

/// A plain preset's place in the order read-only, sandboxed, trusted (design section 6).
/// `None` for custom, which has no place in it.
pub(super) fn rank(preset: Preset) -> Option<u8> {
    match preset {
        Preset::ReadOnly => Some(0),
        Preset::Sandboxed => Some(1),
        Preset::Trusted => Some(2),
        Preset::Custom => None,
    }
}

/// Whether `has` allows, for any input, each tool an allow rule of `wanted` names. Each of the
/// read-only preset's allow rules names a tool alone; an input of nothing matches no allow rule
/// with an argument and every deny or ask rule with one, so `Allow` means allowed everywhere.
fn allows_all(has: &Policy, wanted: &Policy) -> bool {
    wanted
        .rules
        .iter()
        .filter(|rule| rule.action == Action::Allow)
        .all(|rule| has.resolve(&rule.tool, &serde_json::Value::Null) == Action::Allow)
}

/// The list of `uses` that names items of `kind`.
fn names_mut(uses: &mut Uses, kind: LibraryKind) -> &mut Vec<String> {
    match kind {
        LibraryKind::McpServer => &mut uses.mcp_servers,
        LibraryKind::Skill => &mut uses.skills,
        LibraryKind::Plugin => &mut uses.plugins,
        LibraryKind::Context => &mut uses.context,
        LibraryKind::App => &mut uses.apps,
    }
}

/// Whether `grants` gives `consumer` (a key of `Grants`) the variable `name`, in any case.
pub(super) fn grants(grants: &Grants, consumer: &str, name: &str) -> bool {
    grants
        .0
        .get(consumer)
        .is_some_and(|names| names.iter().any(|n| n.eq_ignore_ascii_case(name)))
}

/// `DotSpec::validate`, for a dot whose blank folder is filled when it is created.
fn validate(spec: &DotSpec) -> Result<()> {
    if !spec.workdir.trim().is_empty() {
        return spec.validate();
    }
    let mut filled = spec.clone();
    filled.workdir = ".".to_string();
    filled.validate()
}

/// Puts what `wanted` names on `base`, the dot before it: a new dot with nothing asked, or the
/// made dot as it is. A named part is within reach when the maker has it, or `base` has it
/// already.
fn check(maker: &Dot, base: &DotSpec, wanted: &Wanted, worktrees_root: &Path) -> Result<Reach> {
    let m = &maker.spec;
    let mut spec = base.clone();
    let mut without = base.clone();
    let mut extras = Vec::new();
    let mut create_folder = None;

    // The maker's to write, or not part of reach.
    // Kept without the spaces around it, as the name is checked.
    if let Some(name) = &wanted.name {
        spec.name = name.trim().to_string();
        without.name = spec.name.clone();
    }
    if let Some(instructions) = &wanted.instructions {
        spec.instructions = instructions.clone();
        without.instructions = instructions.clone();
    }
    if let Some(model) = &wanted.model {
        spec.model = model.clone();
        without.model = model.clone();
    }
    if let Some(schedule) = &wanted.schedule {
        spec.schedule = schedule.clone();
        without.schedule = schedule.clone();
    }

    if let Some(uses) = &wanted.uses {
        let mut within = Uses::default();
        for kind in LibraryKind::ALL {
            for name in uses.names(*kind) {
                if m.uses.names(*kind).contains(name) || base.uses.names(*kind).contains(name) {
                    names_mut(&mut within, *kind).push(name.clone());
                } else {
                    extras.push(Extra::Item {
                        item_kind: *kind,
                        name: name.clone(),
                    });
                }
            }
        }
        spec.uses = uses.clone();
        without.uses = within;
    }

    if let Some(variables) = &wanted.variables {
        let (mut asked, mut within) = (Grants::default(), Grants::default());
        for (consumer, names) in &variables.0 {
            // A consumer with no name is granted nothing.
            if names.is_empty() {
                continue;
            }
            asked.0.insert(consumer.clone(), names.clone());
            let mut kept = Vec::new();
            for name in names {
                if grants(&m.variables, consumer, name) || grants(&base.variables, consumer, name) {
                    kept.push(name.clone());
                } else {
                    extras.push(Extra::Variable {
                        consumer: consumer.clone(),
                        name: name.clone(),
                    });
                }
            }
            if !kept.is_empty() {
                within.0.insert(consumer.clone(), kept);
            }
        }
        spec.variables = asked;
        without.variables = within;
    }

    match wanted.permissions {
        None => {}
        Some(WantedPermissions::Maker) => {
            spec.policy = m.policy.clone();
            without.policy = m.policy.clone();
        }
        Some(WantedPermissions::Preset(preset)) => {
            let Some(asked) = rank(preset) else {
                return Err(Error::Invalid(format!("{PERMISSIONS_RULE}.")));
            };
            let no_stronger_than = |has: Preset| rank(has).is_some_and(|has| asked <= has);
            // `Policy::preset` carries the allow rules of Dots' own tools.
            spec.policy = Policy::preset(preset);
            // Custom rules can be narrower than read-only, so a custom policy gives read-only
            // only when it allows all that read-only allows.
            let custom_gives = |has: &Policy| {
                has.preset == Preset::Custom && asked == 0 && allows_all(has, &spec.policy)
            };
            if no_stronger_than(m.policy.preset)
                || no_stronger_than(base.policy.preset)
                || custom_gives(&m.policy)
                || custom_gives(&base.policy)
            {
                without.policy = spec.policy.clone();
            } else {
                extras.push(Extra::Preset {
                    preset,
                    makers: m.policy.preset,
                });
                // Lowered to the maker's own policy, unless that would take from a dot that
                // has more already, or the two cannot be ordered.
                if let (Some(makers), Some(has)) = (rank(m.policy.preset), rank(base.policy.preset))
                {
                    if makers > has {
                        without.policy = m.policy.clone();
                    }
                }
            }
        }
    }

    let folder = wanted.folder.as_deref().map(str::trim).unwrap_or_default();
    if !folder.is_empty() {
        let placed = place(maker, folder, worktrees_root)?;
        let has = base.workdir.trim();
        let same = !has.is_empty() && folded(&real_path(Path::new(has))) == folded(&placed.real);
        // The folder the dot has already stays as it is, mode included.
        if !same {
            spec.workdir = placed.workdir;
            spec.workspace_mode = placed.mode;
            create_folder = placed.create;
            match placed.extra {
                Some(extra) => extras.push(extra),
                None => {
                    without.workdir = spec.workdir.clone();
                    without.workspace_mode = spec.workspace_mode;
                }
            }
        }
    }

    if let Some(on) = wanted.browser {
        spec.browser = on;
        if on && !m.browser && !base.browser {
            extras.push(Extra::Browser);
        } else {
            without.browser = on;
        }
    }
    if let Some(on) = wanted.use_user_settings {
        spec.use_user_settings = on;
        if on && !m.use_user_settings && !base.use_user_settings {
            extras.push(Extra::UserSettings);
        } else {
            without.use_user_settings = on;
        }
    }

    if let Some(turns) = wanted.max_turns {
        let reach = m.max_turns.max(base.max_turns);
        spec.max_turns = turns;
        without.max_turns = turns.min(reach);
        if turns > reach {
            extras.push(Extra::MaxTurns {
                wanted: turns,
                makers: m.max_turns,
            });
        }
    }
    if let Some(secs) = wanted.timeout_secs {
        let reach = m.timeout_secs.max(base.timeout_secs);
        spec.timeout_secs = secs;
        without.timeout_secs = secs.min(reach);
        if secs > reach {
            extras.push(Extra::Timeout {
                wanted_secs: secs,
                makers_secs: m.timeout_secs,
            });
        }
    }

    validate(&spec)?;
    validate(&without)?;
    Ok(Reach {
        spec,
        without,
        extras,
        create_folder,
    })
}

/// For a new dot (design sections 6 and 7.1). `worktrees_root` is where runs' worktree copies
/// are made (`RunnerConfig::worktrees_dir`): the maker's settings do not say.
///
/// Asked nothing but a name and instructions, the dot has the maker's engine, endpoint, model,
/// policy and limits and the settings the table says are copied; a new folder of its own; no
/// Library item, no variable, no browser, no user settings and no schedule; "Can make dots"
/// off and no spending cap of its own; and it is enabled. Its account and workspace are the
/// maker's, which the caller passes on.
///
/// `Error::Invalid` with a plain sentence, for what no card can allow: no name or no
/// instructions, a folder inside a run's worktree copy, a folder outside the maker's that does
/// not exist, and a dot that is not valid (`DotSpec::validate`).
pub fn reach_new(maker: &Dot, wanted: &Wanted, worktrees_root: &Path) -> Result<Reach> {
    let named = |text: &Option<String>| text.as_deref().is_some_and(|t| !t.trim().is_empty());
    if !named(&wanted.name) {
        return Err(Error::Invalid("A new dot needs a name.".into()));
    }
    if !named(&wanted.instructions) {
        return Err(Error::Invalid("A new dot needs instructions.".into()));
    }
    let m = &maker.spec;
    let base = DotSpec {
        name: String::new(),
        instructions: String::new(),
        engine: m.engine,
        model: m.model.clone(),
        endpoint_url: m.endpoint_url.clone(),
        // Blank: `DotAdmin::create` makes a new folder, in folder mode.
        workdir: String::new(),
        workspace_mode: WorkspaceMode::Folder,
        schedule: None,
        policy: m.policy.clone(),
        uses: Uses::default(),
        variables: Grants::default(),
        browser: false,
        quiet_limit_secs: m.quiet_limit_secs,
        spend_cap: crate::spend::DotCaps::default(),
        use_global_instructions: m.use_global_instructions,
        check_results: m.check_results,
        use_graph: m.use_graph,
        context_rules: m.context_rules,
        making: Making::default(),
        use_user_settings: false,
        max_turns: m.max_turns,
        timeout_secs: m.timeout_secs,
        approval_wait_secs: m.approval_wait_secs,
        enabled: true,
    };
    check(maker, &base, wanted, worktrees_root)
}

/// For a change to `made`, a dot the maker made (design section 9). Only the fields `wanted`
/// names change; the rest stays, including what the person edited and the extras they allowed.
/// Only the named fields are checked, against the maker's reach now, and nothing the dot holds
/// already is an extra again: an item or a variable it has, its folder, the browser, a limit no
/// higher and a plain preset no stronger than its own. So removing and lowering never ask.
///
/// That the maker made `made` is the caller's check. Errors as `reach_new`, without the need
/// for a name and instructions.
pub fn reach_change(
    maker: &Dot,
    made: &Dot,
    wanted: &Wanted,
    worktrees_root: &Path,
) -> Result<Reach> {
    check(maker, &made.spec, wanted, worktrees_root)
}

/// Whether `folder`, a folder just made for a dot below the maker's folder, is still within
/// the maker's reach by its real path now (design section 6, folder rules): below the maker's
/// folder, with no dangling link in it, and not in git's or Claude's own folder. A link or
/// junction planted in its path between the check and the making leads it elsewhere.
pub fn still_below_maker(maker: &Dot, folder: &Path) -> bool {
    let own = maker.spec.workdir.trim();
    if !Path::new(own).is_absolute() {
        return false;
    }
    let (real, makers) = (real_path(folder), real_path(Path::new(own)));
    let (key, makers_key) = (folded(&real), folded(&makers));
    if dangling_link(&real) {
        return false;
    }
    key.strip_prefix(&makers_key).is_ok_and(|tail| {
        !tail
            .components()
            .any(|part| settings_part(part.as_os_str()))
    })
}

/// What `made`, a dot the maker made, holds that is beyond the maker's reach as the maker is
/// now (design section 9): what the person allowed it, or gave it since, or what the maker
/// itself no longer has. `own_folder` is whether the dot's folder is the new folder of its own
/// it was made with, which is within reach.
pub fn held_extras(maker: &Dot, made: &DotSpec, own_folder: bool) -> Vec<Extra> {
    let m = &maker.spec;
    let mut extras = Vec::new();
    for kind in LibraryKind::ALL {
        for name in made.uses.names(*kind) {
            if !m.uses.names(*kind).contains(name) {
                extras.push(Extra::Item {
                    item_kind: *kind,
                    name: name.clone(),
                });
            }
        }
    }
    for (consumer, names) in &made.variables.0 {
        for name in names {
            if !grants(&m.variables, consumer, name) {
                extras.push(Extra::Variable {
                    consumer: consumer.clone(),
                    name: name.clone(),
                });
            }
        }
    }
    if made.policy != m.policy {
        let preset = made.policy.preset;
        // A plain preset, as a made dot asks for one, no stronger than the maker's.
        let plain = rank(preset).is_some() && made.policy == Policy::preset(preset);
        let within = plain
            && (rank(m.policy.preset).is_some_and(|has| rank(preset) <= Some(has))
                || (m.policy.preset == Preset::Custom
                    && preset == Preset::ReadOnly
                    && allows_all(&m.policy, &made.policy)));
        if !within {
            extras.push(Extra::Preset {
                preset,
                makers: m.policy.preset,
            });
        }
    }
    if !own_folder {
        extras.extend(held_folder(maker, made));
    }
    if made.browser && !m.browser {
        extras.push(Extra::Browser);
    }
    if made.use_user_settings && !m.use_user_settings {
        extras.push(Extra::UserSettings);
    }
    if made.max_turns > m.max_turns {
        extras.push(Extra::MaxTurns {
            wanted: made.max_turns,
            makers: m.max_turns,
        });
    }
    if made.timeout_secs > m.timeout_secs {
        extras.push(Extra::Timeout {
            wanted_secs: made.timeout_secs,
            makers_secs: m.timeout_secs,
        });
    }
    extras
}

/// The folder of `made` as an extra, when it is beyond the maker's reach now: outside the
/// maker's folder, in git's or Claude's own folder, or below the folder of a maker that works
/// in worktree copies. The maker's own folder is within reach.
fn held_folder(maker: &Dot, made: &DotSpec) -> Option<Extra> {
    let workdir = made.workdir.trim();
    if workdir.is_empty() {
        return None;
    }
    let own = maker.spec.workdir.trim();
    let anchored = Path::new(own).is_absolute();
    let real = real_path(Path::new(workdir));
    let path = real.to_string_lossy().into_owned();
    let (key, makers_key) = (folded(&real), folded(&real_path(Path::new(own))));
    if anchored && key == makers_key {
        return None;
    }
    if anchored && key.starts_with(&makers_key) && !dangling_link(&real) {
        let settings = key.strip_prefix(&makers_key).is_ok_and(|tail| {
            tail.components()
                .any(|part| settings_part(part.as_os_str()))
        });
        return if settings {
            Some(Extra::SettingsFolder { path })
        } else {
            (maker.spec.workspace_mode == WorkspaceMode::Worktree).then_some(Extra::Folder {
                path,
                below_maker: true,
            })
        };
    }
    Some(Extra::Folder {
        path,
        below_maker: false,
    })
}
