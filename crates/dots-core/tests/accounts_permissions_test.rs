//! What each account role may do (members spec §6.3), and what each kind of access to an
//! account allows: Personal is Owner, a `legacy` account acts as Member, and a PC that waits or
//! was removed may do nothing.

use dots_core::accounts::{can, Access, Permission, Role};

const ALL: [Permission; 10] = [
    Permission::View,
    Permission::EditDots,
    Permission::RunDots,
    Permission::AnswerApprovals,
    Permission::GrantEntries,
    Permission::AddMcp,
    Permission::CreateApiKeys,
    Permission::ManageMembers,
    Permission::ManageWorkspaces,
    Permission::SignRoster,
];

#[test]
fn can_matrix() {
    use Permission::*;
    // Viewer, Member, Admin, Owner: the columns of the §6.3 table.
    let table = [
        (View, [true, true, true, true]),
        (EditDots, [false, true, true, true]),
        (RunDots, [false, true, true, true]),
        (AnswerApprovals, [false, true, true, true]),
        (GrantEntries, [false, true, true, true]),
        (AddMcp, [false, true, true, true]),
        (CreateApiKeys, [false, true, true, true]),
        (ManageMembers, [false, false, true, true]),
        (ManageWorkspaces, [false, false, true, true]),
        (SignRoster, [false, false, true, true]),
    ];
    assert_eq!(table.len(), ALL.len());
    let roles = [Role::Viewer, Role::Member, Role::Admin, Role::Owner];
    for (p, row) in table {
        for (role, expected) in roles.into_iter().zip(row) {
            assert_eq!(can(role, p), expected, "{role:?} {p:?}");
        }
    }
}

#[test]
fn each_permission_names_its_action() {
    let actions: Vec<&str> = ALL.iter().map(|p| p.action()).collect();
    assert_eq!(
        actions,
        [
            "see this account's dots",
            "create or change dots",
            "run dots",
            "answer approvals",
            "give secrets to dots",
            "add MCP servers",
            "create API keys",
            "manage members",
            "manage workspaces",
            "sign the member list",
        ]
    );
}

#[test]
fn access_allows_as_its_role() {
    for p in ALL {
        assert!(Access::Owner.allows(p), "{p:?}");
        assert_eq!(Access::Legacy.allows(p), can(Role::Member, p), "{p:?}");
        assert!(!Access::Waiting.allows(p), "{p:?}");
        assert!(!Access::Removed.allows(p), "{p:?}");
        for role in [Role::Viewer, Role::Member, Role::Admin, Role::Owner] {
            assert_eq!(Access::Role(role).allows(p), can(role, p), "{role:?} {p:?}");
        }
    }
}
