use super::super::*;
use super::fake::*;

#[tokio::test]
async fn an_adopted_conversation_is_listed_once_as_its_lane_session() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    let id = {
        let store = f.service.store.lock();
        let record = store.add_session(lane.id, "fake").unwrap();
        store.set_harness_ref(record.id, "s1", "s1").unwrap();
        record.id
    };
    let key = panoptes_key(id);
    let listing = f.service.list().await;
    assert_eq!(listing.sessions.len(), 1, "{:?}", listing.sessions);
    assert_eq!(listing.sessions[0].key, key);
    assert_eq!(listing.sessions[0].title, "One");
    assert_eq!(listing.navigation.len(), 1);
    assert_eq!(listing.navigation[0].lane_id, lane.id);
    assert_eq!(listing.navigation[0].sessions[0].key, key);

    f.service.set_pinned(key.clone(), true).await.unwrap();
    f.service.rename(&key, Some("Login".into())).await.unwrap();
    f.service.set_tags(&key, vec!["auth".into()]).await.unwrap();
    let session = &f.service.list().await.sessions[0];
    assert!(session.pinned);
    assert_eq!(session.title, "Login");
    assert_eq!(session.tags, ["auth"]);
    let pins = Store::open(f._dir.path()).unwrap().pinned().unwrap();
    assert!(pins.is_empty(), "{pins:?}");
    let error = f.service.set_archived("fake:s1", true).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Rejected);

    f.service.set_archived(&key, true).await.unwrap();
    let listing = f.service.list().await;
    assert!(listing.sessions[0].archived, "All sessions keeps it");
    assert!(listing.navigation[0].sessions.is_empty());
    assert_eq!(listing.navigation[0].archived_sessions[0].key, key);
}

#[tokio::test]
async fn adopting_uses_a_unique_worktree_or_asks_for_a_lane() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    f.service.set_pinned("fake:s1".into(), true).await.unwrap();

    let outcome = f
        .service
        .adopt(
            "fake",
            "fake:s1",
            "s1".into(),
            lane.worktrees[0].path.clone(),
            None,
        )
        .await
        .unwrap();
    let AdoptionOutcome::Adopted { key } = outcome else {
        panic!("a unique worktree should adopt directly")
    };
    let listing = f.service.list().await;
    assert_eq!(listing.sessions.len(), 1);
    assert_eq!(listing.sessions[0].key, key);
    assert!(listing.sessions[0].pinned);
    assert!(
        Store::open(f._dir.path())
            .unwrap()
            .pinned()
            .unwrap()
            .is_empty()
    );
    assert_eq!(listing.navigation[0].sessions[0].key, key);

    let outcome = f
        .service
        .adopt(
            "fake",
            "fake:s2",
            "s2".into(),
            lane.worktrees[0].path.clone(),
            None,
        )
        .await
        .unwrap();
    assert!(matches!(outcome, AdoptionOutcome::ChooseLane));
    let outcome = f
        .service
        .adopt(
            "fake",
            "fake:s2",
            "s2".into(),
            "/somewhere/else".into(),
            None,
        )
        .await
        .unwrap();
    assert!(matches!(outcome, AdoptionOutcome::ChooseLane));
    f.service
        .adopt(
            "fake",
            "fake:s2",
            "s2".into(),
            "/somewhere/else".into(),
            Some(lane.id),
        )
        .await
        .unwrap();
    let listing = f.service.list().await;
    assert_eq!(listing.sessions.len(), 2);
    assert_eq!(listing.navigation[0].sessions.len(), 2);
}

#[tokio::test]
async fn pins_are_saved_to_the_store() {
    let f = fixture(FakeHarness::default());
    f.service.set_pinned("fake:s1".into(), true).await.unwrap();
    assert!(f.service.list().await.sessions[0].pinned);
    let saved = Store::open(f._dir.path()).unwrap().pinned().unwrap();
    assert!(saved.contains("fake:s1"));

    f.service.set_pinned("fake:s1".into(), false).await.unwrap();
    assert!(!f.service.list().await.sessions[0].pinned);
}

#[tokio::test]
async fn unreadable_pins_are_reported_and_the_sessions_still_listed() {
    let f = fixture(FakeHarness::default());
    f.service.set_pinned("fake:s1".into(), true).await.unwrap();
    rusqlite::Connection::open(f._dir.path().join("panoptes.db"))
        .unwrap()
        .execute_batch("DROP TABLE pins;")
        .unwrap();

    let listing = f.service.list().await;
    assert_eq!(listing.sessions.len(), 1);
    assert!(!listing.sessions[0].pinned);
    assert_eq!(listing.errors.len(), 1);
    assert!(
        listing.errors[0].starts_with("Could not read pins:"),
        "{:?}",
        listing.errors
    );
}

#[tokio::test]
async fn sessions_are_matched_to_projects_by_their_folder() {
    let code = tempfile::tempdir().unwrap();
    let code = std::fs::canonicalize(code.path()).unwrap();
    let repo = code.join("web");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    git(&repo, &["init", "--quiet", "--initial-branch", "trunk"]);
    git(
        &repo,
        &["commit", "--quiet", "--allow-empty", "-m", "start"],
    );
    let worktree = code.join("workspaces/fix/web");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "fix",
            worktree.to_str().unwrap(),
        ],
    );
    let other = code.join("other");
    std::fs::create_dir_all(&other).unwrap();
    let path = |p: &Path| p.to_string_lossy().into_owned();
    let f = fixture(FakeHarness {
        cwds: vec![path(&repo.join("src")), path(&worktree), path(&other)],
        ..FakeHarness::default()
    });

    // Registering from a worktree registers the main checkout.
    let project = f.service.add_project(path(&worktree)).await.unwrap();
    assert_eq!(project.repo, path(&repo));
    assert_eq!(project.settings.name, "web");
    assert_eq!(project.settings.base_branch, "trunk");
    assert_eq!(project.settings.copy_files, [".env"]);
    let again = f.service.add_project(path(&repo)).await.unwrap_err();
    assert!(again.message.contains("already registered"), "{again}");

    let listing = f.service.list().await;
    assert_eq!(listing.projects, std::slice::from_ref(&project));
    let matched: Vec<_> = listing.sessions.iter().map(|s| s.project_id).collect();
    assert_eq!(matched, [Some(project.id), Some(project.id), None]);
    assert_eq!(listing.sessions[1].project_root, path(&worktree));
    assert_eq!(listing.sessions[1].checkout, Some(path(&repo)));
    assert_eq!(listing.sessions[2].checkout, None);

    f.service.remove_project(project.id).await.unwrap();
    let listing = f.service.list().await;
    assert!(listing.projects.is_empty());
    assert!(listing.sessions.iter().all(|s| s.project_id.is_none()));
}

#[tokio::test]
async fn project_settings_are_checked_before_they_are_saved() {
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "--quiet"]);
    let f = fixture(FakeHarness::default());
    let project = f
        .service
        .add_project(repo.path().to_string_lossy().into_owned())
        .await
        .unwrap();

    let mut settings = project.settings.clone();
    settings.copy_files = vec!["../outside".into()];
    let error = f
        .service
        .update_project(project.id, settings)
        .await
        .unwrap_err();
    assert!(error.message.contains("not a path inside"), "{error}");

    let mut settings = project.settings.clone();
    settings.setup = Some(" pnpm install ".into());
    let saved = f
        .service
        .update_project(project.id, settings)
        .await
        .unwrap();
    assert_eq!(saved.settings.setup.as_deref(), Some("pnpm install"));
    assert_eq!(f.service.list().await.projects, [saved]);

    let outside = tempfile::tempdir().unwrap();
    let error = f
        .service
        .add_project(outside.path().to_string_lossy().into_owned())
        .await
        .unwrap_err();
    assert!(error.message.contains("not in a git repository"), "{error}");
}

#[tokio::test]
async fn lanes_exist_without_sessions_and_dirty_archives_wait_for_a_choice() {
    let code = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(code.path()).unwrap().join("web");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--quiet", "--initial-branch", "trunk"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    std::fs::write(repo.join("tracked.txt"), "start\n").unwrap();
    std::fs::write(repo.join(".env"), "secret\n").unwrap();
    git(&repo, &["add", "tracked.txt"]);
    git(&repo, &["commit", "--quiet", "-m", "start"]);
    let f = fixture(FakeHarness::default());
    let project = f
        .service
        .add_project(repo.to_string_lossy().into_owned())
        .await
        .unwrap();
    let mut settings = project.settings.clone();
    settings.setup = Some("printf ready > setup.txt".into());
    f.service
        .update_project(project.id, settings)
        .await
        .unwrap();

    let lane = f
        .service
        .create_lane(project.id, " Fix login! ".into(), false)
        .await
        .unwrap();
    assert_eq!(lane.name, "fix-login");
    assert_eq!(lane.task, "Fix login!");
    let path = PathBuf::from(&lane.worktrees[0].path);
    assert_eq!(
        std::fs::read_to_string(path.join(".env")).unwrap(),
        "secret\n"
    );
    assert_eq!(
        std::fs::read_to_string(path.join("setup.txt")).unwrap(),
        "ready"
    );
    assert_eq!(f.service.list().await.lanes, std::slice::from_ref(&lane));

    let second = f
        .service
        .create_lane(project.id, "Fix login".into(), false)
        .await
        .unwrap();
    assert_eq!(second.name, "fix-login-2");
    std::fs::write(path.join("tracked.txt"), "work\n").unwrap();
    let outcome = f.service.archive_lane(lane.id, None).await.unwrap();
    assert!(matches!(outcome, ArchiveLaneOutcome::Dirty { .. }));
    assert!(path.exists());
    assert_eq!(f.service.list().await.lanes[0].state, LaneState::Active);

    let outcome = f
        .service
        .archive_lane(lane.id, Some(ArchiveStrategy::Checkpoint))
        .await
        .unwrap();
    assert!(matches!(outcome, ArchiveLaneOutcome::Archived));
    assert!(!path.exists());
    let archived = &f.service.list().await.lanes[0];
    assert_eq!(archived.state, LaneState::Archived);
    assert!(archived.worktrees[0].checkpoint_ref.is_some());

    let restored = f.service.restore_lane(lane.id).await.unwrap();
    assert_eq!(restored.state, LaneState::Active);
    assert_eq!(
        std::fs::read_to_string(path.join("tracked.txt")).unwrap(),
        "work\n"
    );
}
