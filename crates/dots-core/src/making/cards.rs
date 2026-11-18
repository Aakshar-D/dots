//! What the app shows of making (dots-make-dots design section 11): the cards of a maker's chat
//! topic, built from the makers' log with what each card says, and the sentence that says what
//! a made dot uses. They are built here, in the core, so every screen words them the same.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::jobs::{JobEnd, JobStatus, MakingJob};
use super::log::{MakingEvent, MakingEventKind};
use super::manage::schedule_words;
use super::reach::{folded, real_path};
use super::requests::MakeRequestCard;
use super::service::Makers;
use crate::library::LibraryKind;
use crate::model::{Dot, DotSpec};
use crate::vault::Consumer;
use crate::{Error, Result};

/// One card of a maker's chat topic (design section 11): one thing the maker did there, in the
/// order things happened. In JSON the fields of `what` lie beside the others, and `kind` says
/// which card it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MakingCard {
    /// The id of the log's entry the card is built from: it stays the same from read to read.
    pub id: String,
    /// The maker.
    pub maker_dot_id: String,
    /// The maker's name now, for "made by <maker>" and "<maker> changed <name>".
    pub maker_name: String,
    /// The maker's run that did it: the card shows after that run's message.
    pub run_id: String,
    /// The maker's chat topic.
    pub root_run_id: String,
    /// The made dot the card is about, while it exists: `None` says the dot is gone, and for
    /// an ask for a new dot, that there is none yet.
    pub dot_id: Option<String>,
    /// The made dot's name when it happened; for an ask for a new dot, the name asked for.
    pub dot_name: String,
    pub created_at: String,
    #[serde(flatten)]
    pub what: CardWhat,
}

/// What a card says besides, by its kind (design section 11).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum CardWhat {
    /// "Made dot: <name>". While the dot exists: its purpose and schedule as they are now, and
    /// what it uses (`uses_line`). Once it is gone: the purpose it was made with, and no
    /// schedule and no `uses`.
    Made {
        purpose: String,
        /// The schedule in plain words (`schedule_words`); `None` without one.
        schedule: Option<String>,
        uses: Option<String>,
    },
    /// The ask card (design section 7.2). `card` is the whole card as it is now, waiting or
    /// answered (`Makers::request_card`); `None` when the request is gone: the made dot a
    /// change was asked for was deleted.
    Asked {
        request_id: String,
        card: Option<MakeRequestCard>,
    },
    /// "<maker> changed <name>", with the settings that changed, in plain words.
    Changed { what: Vec<String> },
    /// "<maker> deleted <name>".
    Deleted,
    /// "<name> is working" (design section 8).
    JobStarted(JobCard),
    /// "<name> is waiting for your answer".
    JobWaiting(JobCard),
    /// "<name> finished", failed or was stopped: `how` says which.
    JobEnded(JobCard),
}

/// What a job's card says (design section 8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobCard {
    pub job_id: String,
    /// The job's text.
    pub message: String,
    /// The made dot's run the card opens: the run the job started, the run that asked the
    /// person, or the job's last run.
    pub job_run_id: Option<String>,
    /// Whether that run still exists: a made dot's runs go with it.
    pub run_exists: bool,
    /// Where the job stands now, so a card of a job that has moved on can say so; `None` when
    /// the job went with its made dot.
    pub status: Option<JobStatus>,
    /// How the job ended, once it has; `None` for a job that went with its made dot, except
    /// on the card of its end.
    pub how: Option<JobEnd>,
}

/// What a made dot uses, in one line for its card (design section 11): "Uses: MCP server
/// github, folder research/blogs. No secrets." It names the Library items the dot names, by
/// kind, the browser, the person's Claude settings, and its folder; then its variables, each
/// with what it is for. It is built from the dot's settings, which hold the names of variables
/// and never a value. Items switched to "every dot" are not named: every dot has them.
///
/// The folder is named from the maker's when it lies below it ("folder research/blogs"), as
/// "<maker>'s folder" when it is the maker's own, and in full otherwise, or without `maker`.
pub fn uses_line(spec: &DotSpec, maker: Option<&Dot>) -> String {
    let mut parts: Vec<String> = Vec::new();
    for kind in LibraryKind::ALL {
        for name in spec.uses.names(*kind) {
            parts.push(format!("{} {name}", kind.label()));
        }
    }
    if spec.browser {
        parts.push("the browser".to_string());
    }
    if spec.use_user_settings {
        parts.push("your Claude settings".to_string());
    }
    parts.extend(folder_words(spec.workdir.trim(), maker));
    let mut line = if parts.is_empty() {
        "Uses: nothing.".to_string()
    } else {
        format!("Uses: {}.", parts.join(", "))
    };
    let mut variables: Vec<String> = Vec::new();
    for (consumer, names) in &spec.variables.0 {
        let what_for = match Consumer::parse(consumer) {
            Ok(Consumer::Mcp(server)) => format!("the server '{server}'"),
            Ok(Consumer::Shell) => "the shell".to_string(),
            Err(_) => consumer.clone(),
        };
        variables.extend(names.iter().map(|name| format!("{name} for {what_for}")));
    }
    if variables.is_empty() {
        line.push_str(" No secrets.");
    } else {
        line.push_str(&format!(" Variables: {}.", variables.join(", ")));
    }
    line
}

/// The folder `folder` of a made dot in words; `None` for a blank one.
fn folder_words(folder: &str, maker: Option<&Dot>) -> Option<String> {
    if folder.is_empty() {
        return None;
    }
    let makers = maker.filter(|maker| !maker.spec.workdir.trim().is_empty());
    if let Some(maker) = makers {
        let real = real_path(Path::new(folder));
        let makers = real_path(Path::new(maker.spec.workdir.trim()));
        if let Ok(below) = folded(&real).strip_prefix(folded(&makers)) {
            let depth = below.components().count();
            if depth == 0 {
                return Some(format!("{}'s folder", maker.spec.name));
            }
            // The parts below the maker's folder, in the case the folder has.
            let parts: Vec<String> = real
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect();
            let below = parts[parts.len().saturating_sub(depth)..].join("/");
            return Some(format!("folder {below}"));
        }
    }
    Some(format!("folder {folder}"))
}

/// The text `key` of a log entry's detail, when it has one.
fn text(entry: &MakingEvent, key: &str) -> Option<String> {
    entry
        .detail
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

impl Makers {
    /// The cards of the maker's chat topic `root_run_id`, oldest first (design section 11):
    /// one per entry of the topic's log, each with what its card shows, so the chat draws them
    /// with no further call. The person's answer to a request has no card of its own: it shows
    /// on the ask card, which carries the request as it is now. Empty for a topic no maker
    /// did anything in, and once the maker is deleted.
    ///
    /// A waiting request is checked again as its card is built (`request_card`), so reading
    /// the cards may mark a request gone. A made dot, a request or a job that is gone is said
    /// on its card and fails nothing.
    pub async fn cards(&self, root_run_id: &str) -> Result<Vec<MakingCard>> {
        let store = &self.parts.store;
        let entries = store.list_making_events(root_run_id).await?;
        let Some(first) = entries.first() else {
            return Ok(Vec::new());
        };
        let maker = match store.get_dot(&first.maker_dot_id).await {
            Ok(maker) => maker,
            // The maker went since the log was read, and the log with it.
            Err(Error::NotFound(_)) => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let jobs: HashMap<String, MakingJob> = store
            .making_jobs_of_topic(root_run_id)
            .await?
            .into_iter()
            .map(|job| (job.id.clone(), job))
            .collect();
        // A job's text is in the entry of its start; its later cards show it too.
        let messages: HashMap<String, String> = entries
            .iter()
            .filter(|entry| entry.kind == MakingEventKind::JobStarted)
            .filter_map(|entry| Some((text(entry, "job_id")?, text(entry, "message")?)))
            .collect();
        let mut cards = Vec::new();
        for entry in &entries {
            // The made dot as it is now, while it exists.
            let dot = match entry.dot_id.as_deref() {
                Some(id) => match store.get_dot(id).await {
                    Ok(dot) => Some(dot),
                    Err(Error::NotFound(_)) => None,
                    Err(e) => return Err(e),
                },
                None => None,
            };
            let what = match entry.kind {
                // Shown on its ask card.
                MakingEventKind::Answered => continue,
                MakingEventKind::Made => match &dot {
                    Some(dot) => CardWhat::Made {
                        purpose: dot
                            .made
                            .as_ref()
                            .map(|made| made.purpose.clone())
                            .unwrap_or_default(),
                        schedule: dot.spec.schedule.as_ref().map(schedule_words),
                        uses: Some(uses_line(&dot.spec, Some(&maker))),
                    },
                    None => CardWhat::Made {
                        purpose: text(entry, "purpose").unwrap_or_default(),
                        schedule: None,
                        uses: None,
                    },
                },
                MakingEventKind::Asked => {
                    let request_id = text(entry, "request_id").unwrap_or_default();
                    let card = match self.request_card(&request_id).await {
                        Ok(card) => Some(card),
                        Err(Error::NotFound(_)) => None,
                        Err(e) => return Err(e),
                    };
                    CardWhat::Asked { request_id, card }
                }
                MakingEventKind::Changed => CardWhat::Changed {
                    what: entry
                        .detail
                        .get("what")
                        .and_then(|what| serde_json::from_value(what.clone()).ok())
                        .unwrap_or_default(),
                },
                MakingEventKind::Deleted => CardWhat::Deleted,
                MakingEventKind::JobStarted => {
                    CardWhat::JobStarted(self.job_card(entry, &jobs, &messages).await?)
                }
                MakingEventKind::JobWaiting => {
                    CardWhat::JobWaiting(self.job_card(entry, &jobs, &messages).await?)
                }
                MakingEventKind::JobEnded => {
                    CardWhat::JobEnded(self.job_card(entry, &jobs, &messages).await?)
                }
            };
            cards.push(MakingCard {
                id: entry.id.clone(),
                maker_dot_id: entry.maker_dot_id.clone(),
                maker_name: maker.spec.name.clone(),
                run_id: entry.run_id.clone(),
                root_run_id: entry.root_run_id.clone(),
                dot_id: dot.map(|dot| dot.id),
                dot_name: entry.dot_name.clone(),
                created_at: entry.created_at.clone(),
                what,
            });
        }
        Ok(cards)
    }

    /// What the card of the job entry `entry` says: `jobs` are the topic's jobs that exist,
    /// by id, and `messages` the texts of the jobs the topic's log saw start.
    async fn job_card(
        &self,
        entry: &MakingEvent,
        jobs: &HashMap<String, MakingJob>,
        messages: &HashMap<String, String>,
    ) -> Result<JobCard> {
        let job_id = text(entry, "job_id").unwrap_or_default();
        let job = jobs.get(&job_id);
        let job_run_id = text(entry, "run_id");
        let run_exists = match job_run_id.as_deref() {
            Some(id) => match self.parts.store.get_run(id).await {
                Ok(_) => true,
                Err(Error::NotFound(_)) => false,
                Err(e) => return Err(e),
            },
            None => false,
        };
        // The entry of a job's end says how it ended, also once the job is gone.
        let how = text(entry, "how")
            .and_then(|how| JobEnd::parse(&how).ok())
            .or(job.and_then(|job| job.ended_how));
        Ok(JobCard {
            message: messages
                .get(&job_id)
                .cloned()
                .or(job.map(|job| job.message.clone()))
                .unwrap_or_default(),
            job_id,
            job_run_id,
            run_exists,
            status: job.map(|job| job.status),
            how,
        })
    }
}
