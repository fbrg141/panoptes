use super::*;

#[test]
fn restoring_a_lane_forgets_its_checkpoint_and_an_active_lane_can_note_one() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let project = store
        .add_project("/code/app", &settings("App", &[]))
        .unwrap();
    let lane = store
        .add_lane(&NewLane {
            project_id: project.id,
            name: "one".into(),
            task: "Do one".into(),
            branch: "panoptes/one".into(),
            worktrees: vec![NewLaneWorktree {
                project_id: project.id,
                path: "/lanes/one/app".into(),
                main_checkout: false,
            }],
        })
        .unwrap();
    store
        .set_lane_checkpoint(lane.id, Some("refs/panoptes/lanes/one/archive"))
        .unwrap();
    let noted = store.lane(lane.id).unwrap();
    assert_eq!(noted.state, LaneState::Active);
    assert!(noted.worktrees[0].checkpoint_ref.is_some());

    store
        .archive_lane(lane.id, Some("refs/panoptes/lanes/one/archive"))
        .unwrap();
    let restored = store.restore_lane(lane.id).unwrap();
    assert_eq!(restored.worktrees[0].checkpoint_ref, None);
}

#[test]
fn lanes_and_multiple_worktrees_survive_reopening_and_archiving() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let project = store
        .add_project("/code/web", &settings("web", &[]))
        .unwrap();
    let api = store
        .add_project("/code/api", &settings("api", &[]))
        .unwrap();
    let new = NewLane {
        project_id: project.id,
        name: "fix-login".into(),
        task: "Fix login".into(),
        branch: "panoptes/fix-login".into(),
        worktrees: vec![
            NewLaneWorktree {
                project_id: project.id,
                path: "/lanes/fix-login/web".into(),
                main_checkout: false,
            },
            NewLaneWorktree {
                project_id: api.id,
                path: "/lanes/fix-login/api".into(),
                main_checkout: true,
            },
        ],
    };
    let added = store.add_lane(&new).unwrap();
    assert_eq!(added.state, LaneState::Active);
    assert_eq!(added.worktrees.len(), 2);
    drop(store);

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.lanes().unwrap(), std::slice::from_ref(&added));
    let archived = store
        .archive_lane(added.id, Some("refs/panoptes/lanes/1/archive"))
        .unwrap();
    assert_eq!(archived.state, LaneState::Archived);
    assert_eq!(
        archived.worktrees[0].checkpoint_ref.as_deref(),
        Some("refs/panoptes/lanes/1/archive")
    );
    assert_eq!(
        store.restore_lane(added.id).unwrap().state,
        LaneState::Active
    );

    let error = store.remove_project(project.id).unwrap_err().to_string();
    assert!(error.contains("still has lanes"), "{error}");
}

#[test]
fn deleting_a_lane_drops_its_sessions_and_frees_its_project() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let lane = lane_in(&store);
    let session = store.add_session(lane.id, "pi").unwrap();
    store
        .set_session_tags(session.id, &["keep".into(), "local".into()])
        .unwrap();
    assert_eq!(count(&store, "session_tags"), 2);

    store.delete_lane(lane.id).unwrap();

    assert!(store.lanes().unwrap().is_empty());
    assert!(store.sessions().unwrap().is_empty());
    assert_eq!(count(&store, "lane_worktrees"), 0);
    assert_eq!(count(&store, "session_tags"), 0);
    store.remove_project(lane.project_id).unwrap();
    assert!(store.projects().unwrap().is_empty());
}

#[test]
fn lane_names_and_worktree_paths_are_unique() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let project = store
        .add_project("/code/web", &settings("web", &[]))
        .unwrap();
    let new = NewLane {
        project_id: project.id,
        name: "fix-login".into(),
        task: "Fix login".into(),
        branch: "panoptes/fix-login".into(),
        worktrees: vec![NewLaneWorktree {
            project_id: project.id,
            path: "/lanes/fix-login/web".into(),
            main_checkout: false,
        }],
    };
    store.add_lane(&new).unwrap();
    assert!(store.add_lane(&new).is_err());
    let mut other = new.clone();
    other.name = "other".into();
    other.branch = "panoptes/other".into();
    assert!(store.add_lane(&other).is_err());
}

#[test]
fn an_archived_lane_frees_its_path_until_it_is_restored() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let project = store
        .add_project("/code/web", &settings("web", &[]))
        .unwrap();
    let first = NewLane {
        project_id: project.id,
        name: "quick".into(),
        task: "Quick".into(),
        branch: "trunk".into(),
        worktrees: vec![NewLaneWorktree {
            project_id: project.id,
            path: "/code/web".into(),
            main_checkout: true,
        }],
    };
    let added = store.add_lane(&first).unwrap();
    store.archive_lane(added.id, None).unwrap();
    let mut second = first.clone();
    second.name = "quick-2".into();
    store.add_lane(&second).unwrap();
    let error = store.restore_lane(added.id).unwrap_err().to_string();
    assert!(error.contains("already uses"), "{error}");
}
