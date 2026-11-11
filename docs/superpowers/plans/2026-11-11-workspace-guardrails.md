# Workspace guardrails (PR A, core): plan

Spec: `docs/superpowers/specs/2026-11-11-workspace-guardrails-design.md`. The UI (PR B) is built on this branch afterwards.

1. Model: `accounts/guardrails.rs` (`Guardrails`, `GitAction`, defaults, the §3.1 checks, `over_ceiling`), `Workspace.guardrails` in the roster, `parse_roster` checks. Tests first.
2. Permissions: `Permission::ManageGuardrails`, `Permission::ALL`, serde names, the role matrix. Table test.
3. Classifier: `policy/rails.rs` on top of `policy/guard.rs`'s line splitter. Tests first, one per §4.1 case.
4. Access: `accounts::dot_rails` (the role in the dot's workspace and the workspace's guardrails), `Orgs::workspace_guardrails`.
5. Enforcement: `ApprovalHub` (deny for Member, ask for Admin), `RunContext.guardrails`, Claude args and settings, `Runtime::decide_approval` and rule candidates, `approval_guardrail`.
6. Ceiling: saving (`create_dot`, `update_dot`, `move_dot`, approving with a rule), Run now and chat, `Runner::role_hold`, `dot_ceiling`, `policy_ceiling`.
7. Editing: `RosterEdit::SetGuardrails`, `Change::SetGuardrails`, `GuardrailsView`.
8. `dots-admin roster workspace guard`, warnings in `sign`, guardrails in `check`/`verify`.
9. App: the six commands, registration, contract samples, regenerated fixture; `types.ts`, `commands.ts` and their tests.
10. Runner test with `fake-claude`; ceiling hold test.
11. Docs: `docs/accounts.md`, `docs/vault-admin.md` §10, release note line.
12. Gates (fmt, clippy, tests for dots-core, dots-admin, dots-app; UI typecheck and tests), PR.
