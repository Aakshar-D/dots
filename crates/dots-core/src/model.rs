str_enum!(EngineKind { Claude => "claude", Local => "local" });
str_enum!(WorkspaceMode { Worktree => "worktree", Folder => "folder" });
str_enum!(TriggerKind {
    Schedule => "schedule",
    Webhook => "webhook",
    Manual => "manual",
    Chat => "chat",
    Resume => "resume",
});
str_enum!(RunStatus {
    Queued => "queued",
    Running => "running",
    AwaitingApproval => "awaiting_approval",
    Succeeded => "succeeded",
    Failed => "failed",
    Cancelled => "cancelled",
});
str_enum!(ApprovalStatus {
    Pending => "pending",
    Approved => "approved",
    Denied => "denied",
    Expired => "expired",
});

impl RunStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}
