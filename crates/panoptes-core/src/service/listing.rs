use std::collections::HashMap;
use std::path::Path;

use panoptes_harness::{DiscoveredSession, HarnessRef};
use serde::Serialize;

use super::Service;
use super::error::err;
use super::key::{harness_key, panoptes_key};
use super::lanes::lane_cwd;
use super::registry::busy_writer;
use crate::lanes::{self, LaneWork};
use crate::projects::{self, Location};
use crate::store::{Lane, LaneState, Project};

/// The one session type the UI sees: a Panoptes session in a lane, or a
/// harness conversation found on disk that no lane has adopted.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// `panoptes:<id>` or `<harness>:<harness id>`; see [`panoptes_key`].
    pub key: String,
    /// Panoptes' own id; `None` for a conversation no lane has adopted.
    pub id: Option<i64>,
    pub lane_id: Option<i64>,
    pub harness: String,
    /// The harness conversation, once the harness has reported it.
    pub harness_ref: Option<HarnessRef>,
    /// Where it runs: its lane's worktree, or the harness's folder.
    pub cwd: String,
    /// Panoptes' title, else the harness's, else "New session".
    pub title: String,
    pub tags: Vec<String>,
    /// Unix milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
    pub message_count: u32,
    pub parent_locator: Option<String>,
    pub project_root: String,
    /// The main checkout of the git repository the session runs in, shared
    /// by all its worktrees. `None` outside git.
    pub checkout: Option<String>,
    /// For a lane session, its lane's project; otherwise the registered
    /// project whose repository the folder is in.
    pub project_id: Option<i64>,
    pub pinned: bool,
    pub archived: bool,
    pub open: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaneNavigation {
    pub lane_id: i64,
    /// Non-archived sessions, pinned first and then most recent.
    pub sessions: Vec<Session>,
    /// Archived sessions stay attached to the lane for its archive view.
    pub archived_sessions: Vec<Session>,
    /// What an active lane's worktree holds, whether or not it has a
    /// session; `None` for archived lanes or a worktree that can't be read.
    pub work: Option<LaneWork>,
    /// Another session that has edited this lane's worktree and is Running
    /// or Waiting now. While there is one, the lane gets no session of its
    /// own, so there are never two writers.
    pub busy_writer: Option<BusyWriter>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BusyWriter {
    pub session_key: String,
    pub title: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    /// Panoptes sessions, archived ones included, then every harness
    /// conversation no lane has adopted.
    pub sessions: Vec<Session>,
    /// Every registered project, by name, with or without sessions.
    pub projects: Vec<Project>,
    /// Every lane, including archived ones; navigation decides which to show.
    pub lanes: Vec<Lane>,
    /// Authoritative session membership for each lane. The UI renders this
    /// hierarchy rather than inferring it from folders.
    pub navigation: Vec<LaneNavigation>,
    /// One line per harness whose sessions could not be listed, and one
    /// each if the pins, projects, lanes or sessions could not be read.
    pub errors: Vec<String>,
}

impl Service {
    pub async fn list(&self) -> Listing {
        let mut discovered = Vec::new();
        let mut errors = Vec::new();
        for harness in self.harnesses.values() {
            match harness.list_sessions().await {
                Ok(found) => discovered.extend(found),
                Err(error) => errors.push(format!(
                    "Could not list {} sessions: {}",
                    harness.id(),
                    err(error)
                )),
            }
        }
        let pinned = self
            .store
            .call(|store| store.pinned())
            .await
            .unwrap_or_else(|error| {
                errors.push(format!("Could not read pins: {}", err(error)));
                Default::default()
            });
        let projects = self
            .store
            .call(|store| store.projects())
            .await
            .unwrap_or_else(|error| {
                errors.push(format!("Could not read projects: {}", err(error)));
                Vec::new()
            });
        let lanes = self
            .store
            .call(|store| store.lanes())
            .await
            .unwrap_or_else(|error| {
                errors.push(format!("Could not read lanes: {}", err(error)));
                Vec::new()
            });
        let records = self
            .store
            .call(|store| store.sessions())
            .await
            .unwrap_or_else(|error| {
                errors.push(format!("Could not read sessions: {}", err(error)));
                Vec::new()
            });
        let work: HashMap<i64, LaneWork> = {
            let active: Vec<(Project, Lane)> = lanes
                .iter()
                .filter(|lane| lane.state == LaneState::Active)
                .filter_map(|lane| {
                    let project = projects.iter().find(|p| p.id == lane.project_id)?;
                    Some((project.clone(), lane.clone()))
                })
                .collect();
            tokio::task::spawn_blocking(move || {
                active
                    .iter()
                    .filter_map(|(project, lane)| Some((lane.id, lanes::work(project, lane)?)))
                    .collect()
            })
            .await
            .unwrap_or_default()
        };
        let lane_by_id: HashMap<i64, &Lane> = lanes.iter().map(|lane| (lane.id, lane)).collect();
        let by_checkout: HashMap<&Path, i64> = projects
            .iter()
            .map(|project| (Path::new(&project.repo), project.id))
            .collect();
        // Discovered conversations by harness and id, in the harnesses' order.
        let index: HashMap<(&str, &str), usize> = discovered
            .iter()
            .enumerate()
            .map(|(i, session)| {
                let reference = &session.harness_ref;
                ((reference.harness.as_str(), reference.id.as_str()), i)
            })
            .collect();
        let mut adopted = vec![false; discovered.len()];
        let open = self.registry.open().await;
        let busy_writers: HashMap<i64, String> = lanes
            .iter()
            .filter_map(|lane| {
                Some((
                    lane.id,
                    busy_writer(&open, &self.registry.lane_writers(), lane.id)?,
                ))
            })
            .collect();
        let mut locations = self.locations.lock().unwrap();
        let mut sessions = Vec::new();
        for record in records {
            let lane = lane_by_id.get(&record.lane_id).copied();
            let cwd = lane.map(lane_cwd).unwrap_or_default();
            let discovered: Option<&DiscoveredSession> = record
                .harness_id
                .as_deref()
                .and_then(|id| index.get(&(record.harness.as_str(), id)))
                .map(|&i| {
                    adopted[i] = true;
                    &discovered[i]
                });
            let harness_ref = match &discovered {
                Some(session) => Some(session.harness_ref.clone()),
                None => record.harness_id.clone().map(|id| HarnessRef {
                    harness: record.harness.clone(),
                    id,
                    locator: record.locator.clone().unwrap_or_default(),
                    cwd: cwd.clone(),
                }),
            };
            let location = cached_location(&mut locations, &cwd);
            let key = panoptes_key(record.id);
            sessions.push(Session {
                // A preview is open for reading, but is not the lane's live
                // session for navigation, status or session changes.
                open: open.get(&key).is_some_and(|session| !session.preview),
                key,
                id: Some(record.id),
                lane_id: Some(record.lane_id),
                harness: record.harness,
                harness_ref,
                title: record
                    .title
                    .or_else(|| discovered.as_ref().map(|d| d.title.clone()))
                    .unwrap_or_else(|| "New session".into()),
                tags: record.tags,
                created_at: discovered
                    .as_ref()
                    .map_or(record.created_at, |d| d.created_at),
                updated_at: discovered
                    .as_ref()
                    .map_or(record.created_at, |d| d.updated_at),
                message_count: discovered.as_ref().map_or(0, |d| d.message_count),
                parent_locator: discovered.and_then(|d| d.parent_locator.clone()),
                cwd,
                project_root: location.root,
                checkout: location
                    .checkout
                    .as_deref()
                    .map(|c| c.to_string_lossy().into_owned()),
                project_id: lane.map(|lane| lane.project_id),
                pinned: record.pinned,
                archived: record.archived,
            });
        }
        drop(index);
        let unadopted = discovered
            .into_iter()
            .zip(adopted)
            .filter_map(|(session, adopted)| (!adopted).then_some(session));
        for session in unadopted {
            let reference = session.harness_ref;
            let key = harness_key(&reference.harness, &reference.id);
            let location = cached_location(&mut locations, &reference.cwd);
            sessions.push(Session {
                project_id: location
                    .checkout
                    .as_deref()
                    .and_then(|checkout| by_checkout.get(checkout).copied()),
                checkout: location
                    .checkout
                    .as_deref()
                    .map(|c| c.to_string_lossy().into_owned()),
                project_root: location.root,
                pinned: pinned.contains(&key),
                open: open.contains_key(&key),
                key,
                id: None,
                lane_id: None,
                harness: reference.harness.clone(),
                cwd: reference.cwd.clone(),
                harness_ref: Some(reference),
                title: session.title,
                tags: Vec::new(),
                created_at: session.created_at,
                updated_at: session.updated_at,
                message_count: session.message_count,
                parent_locator: session.parent_locator,
                archived: false,
            });
        }
        drop(locations);
        drop(open);
        let all = &sessions;
        let mut navigation: Vec<LaneNavigation> = lanes
            .iter()
            .map(|lane| {
                let mut sessions: Vec<Session> = sessions
                    .iter()
                    .filter(|session| session.lane_id == Some(lane.id))
                    .cloned()
                    .collect();
                sessions.sort_by(|a, b| {
                    b.pinned
                        .cmp(&a.pinned)
                        .then_with(|| b.updated_at.cmp(&a.updated_at))
                });
                let archived_sessions = sessions
                    .iter()
                    .filter(|session| session.archived)
                    .cloned()
                    .collect();
                sessions.retain(|session| !session.archived);
                let busy_writer = busy_writers.get(&lane.id).map(|key| BusyWriter {
                    session_key: key.clone(),
                    title: all
                        .iter()
                        .find(|session| &session.key == key)
                        .map_or_else(|| "Another session".into(), |s| s.title.clone()),
                });
                LaneNavigation {
                    lane_id: lane.id,
                    sessions,
                    archived_sessions,
                    work: work.get(&lane.id).copied(),
                    busy_writer,
                }
            })
            .collect();
        navigation.sort_by_key(|entry| entry.lane_id);
        // All sessions is one reverse-chronological list. The lane lists
        // above apply their own pinned-first ordering.
        sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
        Listing {
            sessions,
            projects,
            lanes,
            navigation,
            errors,
        }
    }
}

fn cached_location(cache: &mut HashMap<String, Location>, cwd: &str) -> Location {
    cache
        .entry(cwd.to_string())
        .or_insert_with(|| projects::locate(Path::new(cwd)))
        .clone()
}
