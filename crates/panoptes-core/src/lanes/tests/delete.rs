use super::*;

#[test]
fn deleting_a_lane_removes_its_dirty_worktree_branch_and_recovery_refs() {
    let f = fixture(None);
    let new = create_lane(&f);
    let mut lane = lane(&f, 1, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::write(path.join("tracked.txt"), "checkpoint work\n").unwrap();
    let ArchiveResult::Archived { checkpoint_ref } =
        archive(&f.project, &lane, Some(ArchiveStrategy::Checkpoint)).unwrap()
    else {
        panic!("checkpoint archive should finish")
    };
    lane.state = LaneState::Archived;
    lane.worktrees[0].checkpoint_ref = checkpoint_ref;
    restore(&f.project, &lane).unwrap();
    std::fs::write(path.join("uncommitted.txt"), "delete me\n").unwrap();

    delete(&f.project, &lane).unwrap();

    assert!(!path.exists());
    assert!(!ref_exists(&f.repo, "refs/heads/panoptes/fix-login").unwrap());
    assert!(!ref_exists(&f.repo, "refs/panoptes/lanes/fix-login/archive").unwrap());
    assert!(!ref_exists(&f.repo, "refs/panoptes/lanes/fix-login/copy-files").unwrap());
    assert_eq!(git(&f.repo, &["branch", "--show-current"]), "trunk");
}

#[test]
fn a_lane_can_be_deleted_after_its_repository_moved() {
    let f = fixture(None);
    let new = create_lane(&f);
    let lane = lane(&f, 1, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::rename(&f.repo, f.repo.with_file_name("moved")).unwrap();

    delete(&f.project, &lane).unwrap();
    assert!(!path.exists());
}

#[test]
fn a_folder_that_is_not_a_worktree_is_kept_when_its_repository_is_gone() {
    let f = fixture(None);
    let new = create_lane(&f);
    let lane = lane(&f, 1, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::remove_file(path.join(".git")).unwrap();
    std::fs::create_dir(path.join(".git")).unwrap();
    std::fs::rename(&f.repo, f.repo.with_file_name("moved")).unwrap();

    delete(&f.project, &lane).unwrap();
    assert!(path.exists());
}

#[test]
fn deleting_a_main_checkout_lane_leaves_the_checkout_and_branch_alone() {
    let f = fixture(None);
    let new = create(
        &f.root,
        &f.project,
        "quick",
        &[],
        "Quick question",
        true,
        &AtomicBool::new(false),
    )
    .unwrap();
    let lane = lane(&f, 1, &new);
    std::fs::write(f.repo.join("uncommitted.txt"), "keep me").unwrap();

    delete(&f.project, &lane).unwrap();

    assert_eq!(
        std::fs::read_to_string(f.repo.join("uncommitted.txt")).unwrap(),
        "keep me"
    );
    assert!(ref_exists(&f.repo, "refs/heads/trunk").unwrap());
}

#[test]
fn deleting_a_lane_on_another_branch_leaves_everything_in_place() {
    let f = fixture(None);
    let new = create_lane(&f);
    let lane = lane(&f, 1, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    git(&path, &["checkout", "--quiet", "-b", "somewhere-else"]);

    let error = delete(&f.project, &lane).unwrap_err();

    assert!(format!("{error:#}").contains("somewhere-else"), "{error:#}");
    assert!(path.exists());
    assert!(ref_exists(&f.repo, "refs/heads/panoptes/fix-login").unwrap());
    assert!(ref_exists(&f.repo, "refs/heads/somewhere-else").unwrap());
}

#[test]
fn deleting_never_counts_the_lane_branch_it_is_about_to_delete() {
    let f = fixture(None);
    let new = create_lane(&f);
    let lane = lane(&f, 1, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::write(path.join("tracked.txt"), "lane-only work\n").unwrap();
    git(&path, &["commit", "--quiet", "-am", "lane-only work"]);
    let commit = git(&path, &["rev-parse", "HEAD"]);
    git(&path, &["checkout", "--quiet", "--detach"]);

    // Only the lane branch holds this commit. Archiving keeps that
    // branch, so the commit is safe there; deleting would remove it.
    let ArchiveResult::DetachedHead(archived) = archive(&f.project, &lane, None).unwrap() else {
        panic!("a detached HEAD should need recovery");
    };
    assert!(archived.is_safe_to_remove());
    let DeleteResult::DetachedHead(deleted) = delete_detached(&f.project, &lane, true).unwrap()
    else {
        panic!("deleting must not treat the lane branch as a safe home");
    };
    assert!(!deleted.is_safe_to_remove());
    assert_eq!(deleted.unreachable_commits[0].hash, commit);
    assert!(path.exists());
    assert_eq!(git(&f.repo, &["rev-parse", "panoptes/fix-login"]), commit);
}

#[test]
fn deleting_a_safe_detached_head_needs_the_explicit_recovery_choice() {
    let f = fixture(None);
    let new = create_lane(&f);
    let lane = lane(&f, 1, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    let commit = git(&path, &["rev-parse", "HEAD"]);
    git(&path, &["checkout", "--quiet", "--detach"]);

    let DeleteResult::DetachedHead(detached) = delete(&f.project, &lane).unwrap() else {
        panic!("a detached HEAD should need recovery");
    };
    assert!(detached.is_safe_to_remove());
    assert!(path.exists());
    std::fs::write(path.join("untracked.txt"), "new work\n").unwrap();
    let DeleteResult::DetachedHead(detached) = delete_detached(&f.project, &lane, true).unwrap()
    else {
        panic!("new work must be refused again");
    };
    assert!(detached.dirty);
    assert!(path.exists());
    std::fs::remove_file(path.join("untracked.txt")).unwrap();

    assert!(matches!(
        delete_detached(&f.project, &lane, true).unwrap(),
        DeleteResult::Deleted
    ));
    assert!(!path.exists());
    assert!(git(&f.repo, &["branch", "--list", "panoptes/fix-login"]).is_empty());
    assert_eq!(git(&f.repo, &["rev-parse", "trunk"]), commit);
}
