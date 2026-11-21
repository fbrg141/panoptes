use super::*;

#[test]
fn a_clean_archive_can_be_restored_on_its_branch() {
    let f = fixture(None);
    let new = create_lane(&f);
    let mut lane = lane(&f, 1, &new);
    archive(&f.project, &lane, None).unwrap();
    lane.state = LaneState::Archived;
    restore(&f.project, &lane).unwrap();
    let path = PathBuf::from(&new.worktrees[0].path);
    assert_eq!(
        git(&path, &["branch", "--show-current"]),
        "panoptes/fix-login"
    );
}

#[test]
fn restore_refuses_a_branch_that_moved_and_leaves_it_alone() {
    let f = fixture(None);
    let new = create_lane(&f);
    let mut lane = lane(&f, 42, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::write(path.join("tracked.txt"), "work\n").unwrap();
    let ArchiveResult::Archived { checkpoint_ref } =
        archive(&f.project, &lane, Some(ArchiveStrategy::Checkpoint)).unwrap()
    else {
        panic!("checkpoint archive should finish")
    };
    lane.worktrees[0].checkpoint_ref = checkpoint_ref;
    lane.state = LaneState::Archived;
    let elsewhere = git(
        &f.repo,
        &[
            "commit-tree",
            "-p",
            "panoptes/fix-login",
            "-m",
            "later",
            "HEAD^{tree}",
        ],
    );
    git(
        &f.repo,
        &["update-ref", "refs/heads/panoptes/fix-login", &elsewhere],
    );

    let error = restore(&f.project, &lane).unwrap_err();
    assert!(format!("{error:#}").contains("new commits"), "{error:#}");
    assert_eq!(
        git(&f.repo, &["rev-parse", "panoptes/fix-login"]),
        elsewhere
    );
    assert!(!path.exists());
}

#[test]
fn a_failed_restore_leaves_the_branch_where_it_was() {
    let f = fixture(None);
    let new = create_lane(&f);
    let mut lane = lane(&f, 42, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::write(path.join("tracked.txt"), "work\n").unwrap();
    let tip = git(&f.repo, &["rev-parse", "panoptes/fix-login"]);
    let ArchiveResult::Archived { checkpoint_ref } =
        archive(&f.project, &lane, Some(ArchiveStrategy::Checkpoint)).unwrap()
    else {
        panic!("checkpoint archive should finish")
    };
    lane.worktrees[0].checkpoint_ref = checkpoint_ref;
    lane.state = LaneState::Archived;
    let mut project = f.project.clone();
    project.settings.setup = Some("exit 3".into());

    assert!(restore(&project, &lane).is_err());
    assert_eq!(git(&f.repo, &["rev-parse", "panoptes/fix-login"]), tip);
    assert!(!path.exists());
}

#[test]
fn a_checkpoint_comes_back_as_uncommitted_work_and_the_branch_stays_put() {
    let f = fixture(None);
    std::fs::write(f.repo.join("doomed.txt"), "remove me\n").unwrap();
    git(&f.repo, &["add", "doomed.txt"]);
    git(&f.repo, &["commit", "--quiet", "-m", "doomed"]);
    let new = create_lane(&f);
    let mut lane = lane(&f, 42, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::write(path.join("tracked.txt"), "work\n").unwrap();
    std::fs::write(path.join("new.txt"), "new\n").unwrap();
    std::fs::remove_file(path.join("doomed.txt")).unwrap();
    let tip = git(&f.repo, &["rev-parse", "panoptes/fix-login"]);

    let result = archive(&f.project, &lane, Some(ArchiveStrategy::Checkpoint)).unwrap();
    let ArchiveResult::Archived { checkpoint_ref } = result else {
        panic!("checkpoint archive should finish")
    };
    lane.worktrees[0].checkpoint_ref = checkpoint_ref;
    lane.state = LaneState::Archived;
    assert!(!path.exists());

    restore(&f.project, &lane).unwrap();
    assert_eq!(
        std::fs::read_to_string(path.join("tracked.txt")).unwrap(),
        "work\n"
    );
    assert_eq!(
        std::fs::read_to_string(path.join("new.txt")).unwrap(),
        "new\n"
    );
    assert!(!path.join("doomed.txt").exists());
    assert_eq!(
        git(&path, &["branch", "--show-current"]),
        "panoptes/fix-login"
    );
    // Nothing was committed on the user's behalf.
    assert_eq!(git(&f.repo, &["rev-parse", "panoptes/fix-login"]), tip);
    let mut status: Vec<String> = git(&path, &["status", "--porcelain"])
        .lines()
        .map(|line| line.trim().to_string())
        .collect();
    status.sort();
    assert_eq!(status, ["?? new.txt", "D doomed.txt", "M tracked.txt"]);
}

#[test]
fn a_checkpoint_restores_onto_a_branch_that_was_deleted() {
    let f = fixture(None);
    let new = create_lane(&f);
    let mut lane = lane(&f, 42, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::write(path.join("tracked.txt"), "work\n").unwrap();
    let tip = git(&f.repo, &["rev-parse", "panoptes/fix-login"]);
    let ArchiveResult::Archived { checkpoint_ref } =
        archive(&f.project, &lane, Some(ArchiveStrategy::Checkpoint)).unwrap()
    else {
        panic!("checkpoint archive should finish")
    };
    lane.worktrees[0].checkpoint_ref = checkpoint_ref;
    lane.state = LaneState::Archived;
    git(&f.repo, &["branch", "-D", "panoptes/fix-login"]);

    restore(&f.project, &lane).unwrap();
    assert_eq!(git(&f.repo, &["rev-parse", "panoptes/fix-login"]), tip);
    assert_eq!(
        std::fs::read_to_string(path.join("tracked.txt")).unwrap(),
        "work\n"
    );
}
