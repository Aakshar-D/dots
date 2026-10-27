//! The review card's preview and the frame's files (app library spec sections 5 and 6.2). A
//! preview runs a proposal's code for real, as the person, on a database in memory built from
//! its migrations and `sample.sql`: calls work, starting dots does not, and its changes vanish
//! when it closes. The frame of an installed app gets the files below `ui/` of the version it
//! was opened at, and nothing else.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use serde_json::Value;

use super::call::{self, Actor, Call, DotRef};
use super::data;
use super::engine::Limits;
use super::propose::{ProposalStatus, DECIDED_ALREADY};
use super::registry::blocking;
use super::source::{self, AppSource};
use super::Apps;
use crate::{Error, Result};

/// The most previews open at once; opening one more closes the oldest.
pub const MAX_PREVIEWS: usize = 8;

/// One open preview.
pub(super) struct Preview {
    id: String,
    proposal_id: String,
    src: Arc<AppSource>,
    /// Taken out while a call runs: one call at a time.
    conn: Mutex<Option<Connection>>,
    /// What `ctx.dots.list()` gives: the dots that use the app now; none for a new app.
    dots: Vec<DotRef>,
}

/// The open previews, oldest first.
#[derive(Clone, Default)]
pub(super) struct Previews(Arc<Mutex<Vec<Arc<Preview>>>>);

impl Previews {
    fn find(&self, id: &str) -> Result<Arc<Preview>> {
        self.0
            .lock()
            .expect("previews")
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("preview {id}")))
    }

    fn add(&self, preview: Preview) {
        let mut open = self.0.lock().expect("previews");
        open.push(Arc::new(preview));
        if open.len() > MAX_PREVIEWS {
            open.remove(0);
        }
    }

    fn close(&self, id: &str) {
        self.0.lock().expect("previews").retain(|p| p.id != id);
    }

    /// Closes the previews of the proposal `proposal_id`: it was decided.
    pub(super) fn close_proposal(&self, proposal_id: &str) {
        self.0
            .lock()
            .expect("previews")
            .retain(|p| p.proposal_id != proposal_id);
    }
}

impl Apps {
    /// Opens a preview of the waiting proposal `proposal_id` (spec section 6.2): its migrations
    /// and `sample.sql` run on a new database in memory. Returns the preview's id. A decided
    /// proposal is a `Conflict`.
    pub async fn preview_open(&self, proposal_id: &str) -> Result<String> {
        let p = self.store.get_app_proposal(proposal_id).await?;
        if p.status != ProposalStatus::Pending {
            return Err(Error::Conflict(DECIDED_ALREADY.into()));
        }
        let dir = self.proposed_dir(&p.id);
        let (src, conn) = blocking(move || -> Result<(AppSource, Connection)> {
            let src = source::read_folder(&dir)?;
            let mut conn = data::open_in_memory()?;
            data::migrate(&mut conn, &src.migrations)?;
            if let Some(sample) = &src.sample {
                data::load_sample(&conn, sample)?;
            }
            Ok((src, conn))
        })
        .await??;
        let dots = match (&p.app_id, self.get(&p.app_name).await) {
            (Some(app_id), Ok((item, _))) if item.id == *app_id => {
                self.listed_dots(&item, &src.manifest).await?
            }
            _ => Vec::new(),
        };
        let id = crate::util::new_id();
        self.previews.add(Preview {
            id: id.clone(),
            proposal_id: p.id,
            src: Arc::new(src),
            conn: Mutex::new(Some(conn)),
            dots,
        });
        Ok(id)
    }

    /// A call from the preview's frame: as the person, only a listed tool, and no dot can be
    /// started. The rule the call breaks is `Invalid`; a closed preview is `NotFound`.
    pub async fn preview_call(&self, preview: &str, tool: &str, input: Value) -> Result<Value> {
        let p = self.previews.find(preview)?;
        let now = self.call_now();
        let function = tool.to_string();
        blocking(move || -> std::result::Result<Value, String> {
            let mut held = p.conn.lock().expect("preview database");
            let conn = held
                .take()
                .ok_or("the preview's database was lost; close the preview and open it again")?;
            let conn = Rc::new(RefCell::new(conn));
            let ran = call::call(
                &conn,
                Call {
                    manifest: &p.src.manifest,
                    logic: &p.src.logic,
                    function: &function,
                    input: &input,
                    actor: Actor::Person,
                    now,
                    dots: p.dots.clone(),
                    start: None,
                    limits: Limits::default(),
                },
            );
            *held = Rc::try_unwrap(conn).ok().map(RefCell::into_inner);
            ran.map(|out| out.result).map_err(|e| e.to_string())
        })
        .await?
        .map_err(Error::Invalid)
    }

    /// Closes the preview: its database and its changes are gone.
    pub fn preview_close(&self, preview: &str) {
        self.previews.close(preview);
    }

    /// The file `path` below `ui/` of the preview's proposal.
    pub fn preview_file(&self, preview: &str, path: &str) -> Result<Vec<u8>> {
        let p = self.previews.find(preview)?;
        ui_file(&p.src, path)
    }

    /// The file `path` below `ui/` of the app `name` at `version`: `NotFound` once another
    /// version is active, so a frame opened at an older one is reloaded, not mixed.
    pub async fn ui_file(&self, name: &str, version: u32, path: &str) -> Result<Vec<u8>> {
        let (item, config) = self.get(name).await?;
        if config.version != version {
            return Err(Error::NotFound(format!(
                "version {version} of the app {name}"
            )));
        }
        let src = self.load(&item, &config).await?;
        ui_file(&src, path)
    }
}

fn ui_file(src: &AppSource, path: &str) -> Result<Vec<u8>> {
    src.ui
        .get(path)
        .cloned()
        .ok_or_else(|| Error::NotFound(format!("ui/{path}")))
}
