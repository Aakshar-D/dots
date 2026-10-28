//! A dot proposes new instructions for itself (conversational chat spec section 4): from a
//! person's chat it calls `propose_instructions`, Dots keeps the proposal and posts a card, and
//! the person's answer continues the chat, as a decided app proposal does.

/// The one name the policy, approvals and grants know `propose_instructions` by. Every preset
/// allows it: nothing changes until the person approves the card.
pub const POLICY_NAME: &str = "Instructions";
