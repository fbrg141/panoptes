use super::super::*;
use super::fake::*;

#[tokio::test]
async fn cleanup_measures_worktrees_and_offers_only_safe_idle_lanes() {
    let f = fixture(FakeHarness::default());
    let (_code, a, b, _sink) = two_lanes(&f).await;
    let c = f
        .service
        .create_lane(a.project_id, "Write the docs".into(), false)
        .await
        .unwrap();
    let main = f
        .service
        .create_lane(a.project_id, "Use the checkout".into(), true)
        .await
        .unwrap();
    let other_repo = _code.path().join("other");
    std::fs::create_dir_all(&other_repo).unwrap();
    git(
        &other_repo,
        &["init", "--quiet", "--initial-branch", "trunk"],
    );
    git(
        &other_repo,
        &["commit", "--quiet", "--allow-empty", "-m", "start"],
    );
    let other_project = f
        .service
        .add_project(other_repo.to_string_lossy().into_owned())
        .await
        .unwrap();
    let _other = f
        .service
        .create_lane(other_project.id, "Another project's lane".into(), false)
        .await
        .unwrap();
    let old = now_ms() - 5 * 24 * 60 * 60 * 1_000;
    for lane in [&a, &b, &c, &main] {
        f.service
            .store
            .lock()
            .unwrap()
            .set_lane_created_at(lane.id, old)
            .unwrap();
    }
    std::fs::write(Path::new(&b.worktrees[0].path).join("dirty.txt"), "keep me").unwrap();

    let cleanup = f.service.lane_cleanup(a.project_id).await.unwrap();
    assert_eq!(cleanup.idle_days, 4);
    assert_eq!(cleanup.project.worktrees, 3);
    assert_eq!(cleanup.total.worktrees, 4);
    assert!(cleanup.project.bytes > 0);
    assert!(cleanup.total.bytes > cleanup.project.bytes);
    assert!(cleanup.errors.is_empty(), "{:?}", cleanup.errors);
    assert_eq!(cleanup.candidates.len(), 2);
    let dirty = cleanup
        .candidates
        .iter()
        .find(|candidate| candidate.lane.id == b.id)
        .unwrap();
    assert_eq!(dirty.preservation, lanes::ArchivePreservation::Checkpoint);
    assert!(dirty.bytes.unwrap() > 0);
    let clean = cleanup
        .candidates
        .iter()
        .find(|candidate| candidate.lane.id == c.id)
        .unwrap();
    assert_eq!(clean.preservation, lanes::ArchivePreservation::Clean);
    // `a` has an open session; the main checkout is not reclaimable.
    assert!(
        cleanup
            .candidates
            .iter()
            .all(|candidate| candidate.lane.id != a.id && candidate.lane.id != main.id)
    );

    git(
        Path::new(&b.worktrees[0].path),
        &["switch", "--quiet", "-c", "somebody-elses-branch"],
    );
    let cleanup = f.service.lane_cleanup(a.project_id).await.unwrap();
    assert_eq!(
        cleanup
            .candidates
            .iter()
            .map(|candidate| candidate.lane.id)
            .collect::<Vec<_>>(),
        [c.id]
    );

    git(
        Path::new(&b.worktrees[0].path),
        &["checkout", "--quiet", "--detach"],
    );
    let detached = f.service.lane_cleanup(a.project_id).await.unwrap();
    assert_eq!(
        detached
            .candidates
            .iter()
            .map(|candidate| candidate.lane.id)
            .collect::<Vec<_>>(),
        [c.id]
    );

    let preferences = f.service.set_cleanup_idle_days(7).await.unwrap();
    assert_eq!(preferences.cleanup_idle_days, 7);
    let cleanup = f.service.lane_cleanup(a.project_id).await.unwrap();
    assert_eq!(cleanup.idle_days, 7);
    assert!(cleanup.candidates.is_empty());
}

#[tokio::test]
async fn cleanup_does_not_guess_idle_activity_when_sessions_cannot_be_listed() {
    let f = fixture(FakeHarness {
        fail_listing: true,
        ..Default::default()
    });
    let (_code, _a, b, _sink) = two_lanes(&f).await;
    f.service
        .store
        .lock()
        .unwrap()
        .set_lane_created_at(b.id, now_ms() - 5 * 24 * 60 * 60 * 1_000)
        .unwrap();

    let cleanup = f.service.lane_cleanup(b.project_id).await.unwrap();
    assert!(cleanup.candidates.is_empty());
    assert!(
        cleanup
            .errors
            .iter()
            .any(|error| error.contains("Could not list fake sessions")),
        "{:?}",
        cleanup.errors
    );
}
