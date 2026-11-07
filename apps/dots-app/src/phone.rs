//! Answer from your phone (phone view spec): the app's side. The listener lives in the core;
//! here it is made with the runtime and the PC's own pairing dialog, and applied at start and
//! whenever Settings are saved.

use std::sync::Arc;

use async_trait::async_trait;
use dots_core::phone::{pair_question, PairApprover, PairRequest, Phone};
use dots_core::vault::org::os_protector;
use dots_core::Runtime;
use tauri::{AppHandle, Manager};

use crate::confirm;
use crate::error::CmdError;

/// The phone view, when the runtime started.
pub struct PhoneHandle(Option<Phone>);

impl PhoneHandle {
    pub fn get(&self) -> Result<&Phone, CmdError> {
        self.0
            .as_ref()
            .ok_or_else(|| CmdError::internal("Dots did not start; see the startup error"))
    }
}

/// Asks the person at the PC in a native dialog, which neither a phone nor the webview can
/// answer, and which no environment variable answers either. The window comes to the front
/// first: a dialog behind a hidden window would go unseen until the claim ran out.
struct Dialog(AppHandle);

#[async_trait]
impl PairApprover for Dialog {
    async fn approve(&self, request: PairRequest) -> bool {
        if let Some(window) = self.0.get_webview_window("main") {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
        let (title, message) = pair_question(&request);
        confirm::ask_in_dialog(&self.0, &title, message).await
    }
}

/// Makes the phone view for `runtime` and applies the stored settings: it listens only when
/// they turn it on.
pub fn manage(app: &AppHandle, runtime: Option<Arc<Runtime>>) {
    let phone = runtime.map(|rt| {
        let phone = Phone::new(
            rt.store().clone(),
            rt.clone(),
            os_protector(),
            Arc::new(Dialog(app.clone())),
        );
        let started = phone.clone();
        tauri::async_runtime::spawn(async move {
            started.apply(&rt.settings().await).await;
        });
        phone
    });
    app.manage(PhoneHandle(phone));
}
