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
