//! This PC's place on an account's signed member list (members spec §6.4). It is worked out
//! after every accepted fetch by the `Orgs` service and kept in the `orgs` row.

str_enum!(Membership {
    Legacy => "legacy",
    Waiting => "waiting",
    Active => "active",
    Removed => "removed",
});

/// An account whose policy asks for no member list is `legacy`, as every account was before
/// member lists (§6.6).
impl Default for Membership {
    fn default() -> Membership {
        Membership::Legacy
    }
}

/// Why this PC gets nothing from an account (members spec §6.4): it waits to be added, its Dots
/// ID changed since it was on the list, or it was removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotMember {
    Waiting,
    /// This PC was on the list, and still is by its old Dots ID or as the same member, but its
    /// current Dots ID is not: an Admin adds the new one. Nothing is wiped.
    IdChanged,
    Removed,
}

impl NotMember {
    /// What a run of the account's dots, and each of their variables, is refused with.
    pub fn text(&self, account: &str) -> String {
        match self {
            Self::Waiting => format!(
                "{account}: you aren't on its member list yet. Send your Dots ID to an Admin."
            ),
            Self::IdChanged => {
                format!("{account}: this PC's Dots ID changed. Send the new one to an Admin.")
            }
            Self::Removed => format!("{account}: you were removed from it."),
        }
    }
}
