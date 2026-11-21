use super::*;

#[test]
fn a_lane_whose_folder_was_deleted_can_still_be_archived() {
    let f = fixture(None);
    let new = create_lane(&f);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::remove_dir_all(&path).unwrap();
    let result = archive(&f.project, &lane(&f, 1, &new), None).unwrap();
    assert!(matches!(result, ArchiveResult::Archived { .. }));
    assert!(!git(&f.repo, &["worktree", "list"]).contains("fix-login"));
}

#[test]
fn archive_restores_lane_copy_files_for_every_strategy() {
    for strategy in [
        None,
        Some(ArchiveStrategy::Commit),
        Some(ArchiveStrategy::Stash),
        Some(ArchiveStrategy::Checkpoint),
    ] {
        let f = fixture(None);
        let new = create_lane(&f);
        let mut lane = lane(&f, 1, &new);
        let path = PathBuf::from(&new.worktrees[0].path);
        std::fs::write(path.join(".env"), "lane secret\n").unwrap();
        if strategy.is_some() {
            std::fs::write(path.join("tracked.txt"), "work\n").unwrap();
        }

        let ArchiveResult::Archived { checkpoint_ref } =
            archive(&f.project, &lane, strategy).unwrap()
        else {
            panic!("archive should finish")
        };
        lane.worktrees[0].checkpoint_ref = checkpoint_ref;
        lane.state = LaneState::Archived;
        std::fs::write(f.repo.join(".env"), "replacement secret\n").unwrap();
        restore(&f.project, &lane).unwrap();

        assert_eq!(
            std::fs::read_to_string(path.join(".env")).unwrap(),
            "lane secret\n",
            "{strategy:?}"
        );
        assert!(
            ref_exists(&f.repo, "refs/panoptes/lanes/fix-login/copy-files").unwrap(),
            "{strategy:?}"
        );
        // A checkpoint comes back as the uncommitted change it was.
        let expected = if strategy == Some(ArchiveStrategy::Checkpoint) {
            "M tracked.txt"
        } else {
            ""
        };
        assert_eq!(
            git(&path, &["status", "--porcelain"]).trim(),
            expected,
            "{strategy:?}"
        );
    }
}

#[test]
fn unconfigured_ignored_files_are_not_preserved() {
    let f = fixture(None);
    let new = create_lane(&f);
    let mut lane = lane(&f, 1, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::write(path.join("scratch.log"), "temporary\n").unwrap();
    archive(&f.project, &lane, None).unwrap();
    lane.state = LaneState::Archived;

    restore(&f.project, &lane).unwrap();

    assert!(!path.join("scratch.log").exists());
}

#[test]
fn non_forced_removal_preserves_a_worktree_that_became_dirty() {
    let f = fixture(None);
    let new = create_lane(&f);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::write(path.join("tracked.txt"), "new work\n").unwrap();

    remove_worktree(&f.repo, &path, false).unwrap_err();

    assert_eq!(
        std::fs::read_to_string(path.join("tracked.txt")).unwrap(),
        "new work\n"
    );
}

#[test]
fn archiving_a_clean_lane_keeps_its_branch() {
    let f = fixture(None);
    let new = create_lane(&f);
    let lane = lane(&f, 1, &new);
    let result = archive(&f.project, &lane, None).unwrap();
    assert!(matches!(
        result,
        ArchiveResult::Archived {
            checkpoint_ref: None
        }
    ));
    assert!(!PathBuf::from(&new.worktrees[0].path).exists());
    assert!(ref_exists(&f.repo, "refs/heads/panoptes/fix-login").unwrap());
}

#[test]
fn a_dirty_archive_without_a_choice_does_nothing() {
    let f = fixture(None);
    let new = create_lane(&f);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::write(path.join("tracked.txt"), "work\n").unwrap();
    let before = git(&path, &["status", "--porcelain"]);

    let result = archive(&f.project, &lane(&f, 1, &new), None).unwrap();
    assert!(matches!(result, ArchiveResult::Dirty));
    assert!(path.exists());
    assert_eq!(git(&path, &["status", "--porcelain"]), before);
}

#[test]
fn commit_archive_refuses_a_nested_repository() {
    let f = fixture(None);
    let new = create_lane(&f);
    let path = PathBuf::from(&new.worktrees[0].path);
    nested_repository(&path.join("inner"));

    let error = archive(
        &f.project,
        &lane(&f, 1, &new),
        Some(ArchiveStrategy::Commit),
    )
    .unwrap_err();

    assert!(format!("{error:#}").contains("inner"));
    assert!(path.join("inner/.git").exists());
    assert_eq!(
        std::fs::read_to_string(path.join("inner/inner.txt")).unwrap(),
        "inner work\n"
    );
    assert_eq!(
        git(&f.repo, &["ls-tree", "panoptes/fix-login", "inner"]),
        ""
    );
}

#[test]
fn checkpoint_archive_refuses_a_nested_repository() {
    let f = fixture(None);
    let new = create_lane(&f);
    let lane = lane(&f, 1, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    nested_repository(&path.join("inner"));

    let error = archive(&f.project, &lane, Some(ArchiveStrategy::Checkpoint)).unwrap_err();

    assert!(format!("{error:#}").contains("inner"));
    assert!(path.join("inner/.git").exists());
    assert_eq!(
        std::fs::read_to_string(path.join("inner/inner.txt")).unwrap(),
        "inner work\n"
    );
    assert!(!ref_exists(&f.repo, "refs/panoptes/lanes/fix-login/archive").unwrap());
}

#[test]
fn snapshots_allow_a_gitlink_already_in_head() {
    let f = fixture(None);
    let new = create_lane(&f);
    let lane = lane(&f, 1, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    let source = f.repo.parent().unwrap().join("submodule-source");
    nested_repository(&source);
    git(
        &path,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            &source.to_string_lossy(),
            "dependency",
        ],
    );
    git(&path, &["commit", "--quiet", "-m", "add dependency"]);

    let tree = snapshot_tree(&path, &lane).unwrap();

    assert!(git(&path, &["cat-file", "-e", &tree]).is_empty());
    assert!(git(&path, &["ls-tree", &tree, "dependency"]).starts_with("160000 "));
}

#[test]
fn commit_archives_all_changes_on_the_lane_branch() {
    let f = fixture(None);
    let new = create_lane(&f);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::write(path.join("tracked.txt"), "work\n").unwrap();
    std::fs::write(path.join("new.txt"), "new\n").unwrap();

    let result = archive(
        &f.project,
        &lane(&f, 1, &new),
        Some(ArchiveStrategy::Commit),
    )
    .unwrap();
    assert!(matches!(result, ArchiveResult::Archived { .. }));
    assert!(!path.exists());
    assert_eq!(
        git(&f.repo, &["show", "panoptes/fix-login:tracked.txt"]),
        "work"
    );
    assert_eq!(git(&f.repo, &["show", "panoptes/fix-login:new.txt"]), "new");
}

#[test]
fn new_changes_from_a_commit_hook_keep_the_worktree_in_place() {
    use std::os::unix::fs::PermissionsExt;

    let f = fixture(None);
    let new = create_lane(&f);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::write(path.join("tracked.txt"), "work\n").unwrap();
    git(&path, &["config", "status.showUntrackedFiles", "no"]);
    let hook = f.repo.join(".git/hooks/post-commit");
    std::fs::write(&hook, "#!/bin/sh\ntouch from-hook.txt\n").unwrap();
    let mut permissions = std::fs::metadata(&hook).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&hook, permissions).unwrap();

    let error = archive(
        &f.project,
        &lane(&f, 1, &new),
        Some(ArchiveStrategy::Commit),
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("worktree was left in place"));
    assert!(path.join("from-hook.txt").exists());
}

#[test]
fn stash_archives_changes_without_committing_them() {
    let f = fixture(None);
    let new = create_lane(&f);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::write(path.join("tracked.txt"), "work\n").unwrap();
    std::fs::write(path.join("new.txt"), "new\n").unwrap();

    let result = archive(&f.project, &lane(&f, 1, &new), Some(ArchiveStrategy::Stash)).unwrap();
    assert!(matches!(result, ArchiveResult::Archived { .. }));
    assert!(!path.exists());
    assert_eq!(
        git(&f.repo, &["show", "panoptes/fix-login:tracked.txt"]),
        "start"
    );
    assert!(git(&f.repo, &["stash", "list"]).contains("Panoptes lane fix-login"));
    let stash = git(
        &f.repo,
        &["stash", "show", "--include-untracked", "--name-only"],
    );
    assert!(stash.contains("new.txt"), "{stash}");
}

#[test]
fn a_checkpoint_preserves_the_real_index_head_and_branch() {
    let f = fixture(None);
    let new = create_lane(&f);
    let lane = lane(&f, 42, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::write(path.join("tracked.txt"), "staged\n").unwrap();
    git(&path, &["add", "tracked.txt"]);
    std::fs::write(path.join("new.txt"), "untracked\n").unwrap();
    let status = git(&path, &["status", "--porcelain"]);
    let head = git(&path, &["rev-parse", "HEAD"]);

    let (reference, _) = checkpoint(&f.project, &lane, &path).unwrap();
    assert_eq!(reference, "refs/panoptes/lanes/fix-login/archive");
    assert_eq!(git(&path, &["status", "--porcelain"]), status);
    assert_eq!(git(&path, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        git(&f.repo, &["show", "panoptes/fix-login:tracked.txt"]),
        "start"
    );
    assert_eq!(
        git(&f.repo, &["show", &format!("{reference}:tracked.txt")]),
        "staged"
    );
    assert_eq!(
        git(&f.repo, &["show", &format!("{reference}:new.txt")]),
        "untracked"
    );
}

#[test]
fn snapshots_see_a_second_edit_to_an_already_modified_file() {
    // `git status` reads ` M tracked.txt` both times; the tree does not.
    let f = fixture(None);
    let new = create_lane(&f);
    let lane = lane(&f, 1, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::write(path.join("tracked.txt"), "first\n").unwrap();
    let before = snapshot_tree(&path, &lane).unwrap();
    let status = git(&path, &["status", "--porcelain"]);
    std::fs::write(path.join("tracked.txt"), "second\n").unwrap();
    assert_eq!(git(&path, &["status", "--porcelain"]), status);
    assert_ne!(snapshot_tree(&path, &lane).unwrap(), before);
}

#[test]
fn an_interrupted_checkpoint_archive_is_found_again() {
    let f = fixture(None);
    let new = create_lane(&f);
    let mut lane = lane(&f, 42, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::write(path.join("tracked.txt"), "work\n").unwrap();
    // The archive records its checkpoint, removes the folder, and then
    // Panoptes stops before the lane is marked archived.
    let recorded = std::cell::RefCell::new(None);
    archive_recording(
        &f.project,
        &lane,
        Some(ArchiveStrategy::Checkpoint),
        &|reference| {
            *recorded.borrow_mut() = reference.map(str::to_string);
            Ok(())
        },
    )
    .unwrap();
    assert!(!path.exists());
    lane.worktrees[0].checkpoint_ref = recorded.into_inner();
    assert!(lane.worktrees[0].checkpoint_ref.is_some());

    // Archiving the still-active lane again keeps the checkpoint.
    let ArchiveResult::Archived { checkpoint_ref } = archive(&f.project, &lane, None).unwrap()
    else {
        panic!("the archive should finish")
    };
    assert_eq!(checkpoint_ref, lane.worktrees[0].checkpoint_ref);
    lane.state = LaneState::Archived;
    restore(&f.project, &lane).unwrap();
    assert_eq!(
        std::fs::read_to_string(path.join("tracked.txt")).unwrap(),
        "work\n"
    );
}
