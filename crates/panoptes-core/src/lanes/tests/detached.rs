use super::*;

#[test]
fn a_detached_head_with_unreachable_commits_is_refused_and_can_be_saved() {
    let f = fixture(None);
    let new = create_lane(&f);
    let lane = lane(&f, 1, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    git(&path, &["checkout", "--quiet", "--detach"]);
    std::fs::write(path.join("tracked.txt"), "detached work\n").unwrap();
    git(&path, &["commit", "--quiet", "-am", "detached work"]);
    let commit = git(&path, &["rev-parse", "HEAD"]);

    for strategy in [None, Some(ArchiveStrategy::Commit)] {
        let ArchiveResult::DetachedHead(detached) = archive(&f.project, &lane, strategy).unwrap()
        else {
            panic!("a detached HEAD should need recovery");
        };
        assert!(!detached.is_safe_to_remove());
        assert_eq!(detached.unreachable_commits.len(), 1);
        assert_eq!(detached.unreachable_commits[0].hash, commit);
        assert_eq!(detached.unreachable_commits[0].subject, "detached work");
    }
    assert!(path.exists());
    assert_eq!(git(&path, &["rev-parse", "HEAD"]), commit);

    create_branch_here(&path, &lane).unwrap();
    assert_eq!(
        git(&path, &["branch", "--show-current"]),
        "panoptes/fix-login"
    );
    assert!(matches!(
        archive(&f.project, &lane, None).unwrap(),
        ArchiveResult::Archived { .. }
    ));
    assert_eq!(git(&f.repo, &["rev-parse", "panoptes/fix-login"]), commit);
}

#[test]
fn a_clean_detached_head_on_a_pushed_commit_can_recreate_its_branch_and_archive() {
    let f = fixture(None);
    let new = create_lane(&f);
    let lane = lane(&f, 1, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::write(path.join("tracked.txt"), "pushed work\n").unwrap();
    git(&path, &["commit", "--quiet", "-am", "pushed work"]);
    let commit = git(&path, &["rev-parse", "HEAD"]);
    git(&path, &["checkout", "--quiet", "--detach"]);
    git(
        &f.repo,
        &["update-ref", "refs/remotes/origin/pushed", &commit],
    );
    git(&f.repo, &["branch", "-D", "panoptes/fix-login"]);

    let ArchiveResult::DetachedHead(detached) = archive(&f.project, &lane, None).unwrap() else {
        panic!("a detached HEAD should need recovery");
    };
    assert!(detached.is_safe_to_remove());
    assert_eq!(detached.commit, commit);
    assert!(path.exists());

    let result = archive_recovering(&f.project, &lane, None, true, &|_| Ok(())).unwrap();
    assert!(matches!(result, ArchiveResult::Archived { .. }));
    assert!(!path.exists());
    assert_eq!(git(&f.repo, &["rev-parse", "panoptes/fix-login"]), commit);
}

#[test]
fn a_dirty_detached_head_is_not_safe_even_when_its_commit_is_pushed() {
    let f = fixture(None);
    let new = create_lane(&f);
    let lane = lane(&f, 1, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    let commit = git(&path, &["rev-parse", "HEAD"]);
    git(&path, &["checkout", "--quiet", "--detach"]);
    git(
        &f.repo,
        &["update-ref", "refs/remotes/origin/pushed", &commit],
    );
    git(&f.repo, &["branch", "-D", "panoptes/fix-login"]);
    std::fs::write(path.join("untracked.txt"), "unsaved\n").unwrap();
    git(&f.repo, &["config", "status.showUntrackedFiles", "no"]);

    let ArchiveResult::DetachedHead(detached) = archive(&f.project, &lane, None).unwrap() else {
        panic!("a detached HEAD should need recovery");
    };
    assert!(detached.dirty);
    assert!(detached.unreachable_commits.is_empty());
    assert!(!detached.is_safe_to_remove());
    assert_eq!(git(&path, &["rev-parse", "HEAD"]), commit);
    assert!(path.join("untracked.txt").exists());
}

#[test]
fn recovery_advances_an_existing_lane_branch_but_never_resets_a_diverged_one() {
    let f = fixture(None);
    let new = create_lane(&f);
    let lane = lane(&f, 1, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    git(&path, &["checkout", "--quiet", "--detach"]);
    std::fs::write(path.join("tracked.txt"), "detached work\n").unwrap();
    git(&path, &["commit", "--quiet", "-am", "detached work"]);
    let commit = git(&path, &["rev-parse", "HEAD"]);
    git(&f.repo, &["branch", "published", &commit]);

    create_branch_here(&path, &lane).unwrap();
    assert_eq!(git(&f.repo, &["rev-parse", "panoptes/fix-login"]), commit);

    let f = fixture(None);
    let new = create_lane(&f);
    let diverged_lane = self::lane(&f, 1, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::write(path.join("tracked.txt"), "lane work\n").unwrap();
    git(&path, &["commit", "--quiet", "-am", "lane work"]);
    let lane_commit = git(&path, &["rev-parse", "HEAD"]);
    git(&path, &["checkout", "--quiet", "--detach", "trunk"]);
    std::fs::write(path.join("tracked.txt"), "detached work\n").unwrap();
    git(&path, &["commit", "--quiet", "-am", "detached work"]);
    let detached_commit = git(&path, &["rev-parse", "HEAD"]);
    git(&f.repo, &["branch", "published", &detached_commit]);

    let error = create_branch_here(&path, &diverged_lane).unwrap_err();
    assert!(format!("{error:#}").contains("was not moved"), "{error:#}");
    assert_eq!(
        git(&f.repo, &["rev-parse", "panoptes/fix-login"]),
        lane_commit
    );
    assert_eq!(git(&path, &["rev-parse", "HEAD"]), detached_commit);
}

#[test]
fn another_branch_is_refused_so_its_commits_are_kept() {
    let f = fixture(None);
    let new = create_lane(&f);
    let path = PathBuf::from(&new.worktrees[0].path);
    git(&path, &["checkout", "--quiet", "-b", "side"]);
    std::fs::write(path.join("tracked.txt"), "side work\n").unwrap();
    git(&path, &["commit", "--quiet", "-am", "side work"]);

    let error = archive(&f.project, &lane(&f, 1, &new), None).unwrap_err();
    assert!(format!("{error:#}").contains("is on side instead of panoptes/fix-login"));
    assert!(path.exists());
}

#[test]
fn an_operation_in_progress_is_refused() {
    let f = fixture(None);
    let new = create_lane(&f);
    let path = PathBuf::from(&new.worktrees[0].path);
    let head = git(&path, &["rev-parse", "HEAD"]);
    std::fs::write(git_path(&path, "MERGE_HEAD").unwrap(), format!("{head}\n")).unwrap();

    let error = archive(&f.project, &lane(&f, 1, &new), None).unwrap_err();
    assert!(format!("{error:#}").contains("in the middle of a merge"));
    assert!(path.exists());
}
