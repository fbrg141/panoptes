use super::*;
use crate::store::NewLaneWorktree;

#[test]
fn work_counts_changed_files_and_commits_ahead_of_the_base() {
    let f = fixture(None);
    let new = create_lane(&f);
    let lane = lane(&f, 1, &new);
    let path = PathBuf::from(&lane.worktrees[0].path);
    let none = LaneWork {
        changed_files: 0,
        commits_ahead: Some(0),
    };
    assert_eq!(work(&f.project, &lane), Some(none));
    std::fs::write(path.join("a.txt"), "a").unwrap();
    std::fs::write(path.join("b.txt"), "b").unwrap();
    git(&path, &["add", "a.txt"]);
    git(&path, &["commit", "--quiet", "-m", "a"]);
    assert_eq!(
        work(&f.project, &lane),
        Some(LaneWork {
            changed_files: 1,
            commits_ahead: Some(1)
        })
    );
}

#[test]
fn untracked_files_count_as_changes_whatever_the_status_settings() {
    let f = fixture(None);
    git(&f.repo, &["config", "status.showUntrackedFiles", "no"]);
    let new = create_lane(&f);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::write(path.join("agent-wrote.txt"), "work\n").unwrap();
    std::fs::create_dir(path.join("nested")).unwrap();
    std::fs::write(path.join("nested/deep.txt"), "work\n").unwrap();

    let result = archive(&f.project, &lane(&f, 1, &new), None).unwrap();
    assert!(matches!(result, ArchiveResult::Dirty));
    assert!(path.join("agent-wrote.txt").exists());
    assert!(path.join("nested/deep.txt").exists());
}

#[test]
fn worktree_status_sees_submodule_changes_hidden_by_config() {
    let f = fixture(None);
    let new = create_lane(&f);
    let path = PathBuf::from(&new.worktrees[0].path);
    let source = f.repo.parent().unwrap().join("submodule-source");
    std::fs::create_dir(&source).unwrap();
    git(&source, &["init", "--quiet", "--initial-branch", "trunk"]);
    std::fs::write(source.join("tracked.txt"), "start\n").unwrap();
    git(&source, &["add", "tracked.txt"]);
    git(&source, &["commit", "--quiet", "-m", "start"]);
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
    let dependency = path.join("dependency");
    std::fs::write(dependency.join("new.txt"), "new\n").unwrap();
    git(&dependency, &["add", "new.txt"]);
    git(&dependency, &["commit", "--quiet", "-m", "new commit"]);
    git(&path, &["config", "diff.ignoreSubmodules", "all"]);
    assert_eq!(git(&path, &["status", "--porcelain"]), "");

    assert!(!worktree_status(&path).unwrap().is_empty());
}

#[test]
fn concurrent_snapshots_use_independent_indexes() {
    let f = fixture(None);
    let new = create_lane(&f);
    let lane = lane(&f, 1, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::write(path.join("tracked.txt"), "change\n").unwrap();
    let expected = snapshot_tree(&path, &lane).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let readers: Vec<_> = (0..8)
        .map(|_| {
            let path = path.clone();
            let lane = lane.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                snapshot_tree(&path, &lane)
            })
        })
        .collect();
    for reader in readers {
        assert_eq!(reader.join().unwrap().unwrap(), expected);
    }
}

#[test]
fn snapshots_retain_large_files() {
    let f = fixture(None);
    let new = create_lane(&f);
    let lane = lane(&f, 1, &new);
    let path = PathBuf::from(&new.worktrees[0].path);
    std::fs::write(path.join("large.txt"), "x".repeat(1_100_000)).unwrap();
    let tree = snapshot_tree(&path, &lane).unwrap();
    let shown = git(&f.repo, &["show", &format!("{tree}:large.txt")]);
    assert_eq!(shown.len(), 1_100_000);
}

#[test]
#[ignore = "benchmark; run explicitly with --ignored --nocapture"]
fn a_no_change_10k_file_snapshot_is_measured() {
    let f = fixture(None);
    let many = f.repo.join("many");
    std::fs::create_dir_all(&many).unwrap();
    for i in 0..10_000 {
        std::fs::write(many.join(format!("{i:05}.txt")), "x").unwrap();
    }
    git(&f.repo, &["add", "-A"]);
    git(&f.repo, &["commit", "--quiet", "-m", "many files"]);
    let new = NewLane {
        project_id: f.project.id,
        name: "bench".into(),
        task: "Benchmark".into(),
        branch: "trunk".into(),
        worktrees: vec![NewLaneWorktree {
            project_id: f.project.id,
            path: f.repo.to_string_lossy().into_owned(),
            main_checkout: true,
        }],
    };
    let lane = lane(&f, 99, &new);
    let started = std::time::Instant::now();
    let first = snapshot_tree(&f.repo, &lane).unwrap();
    let before = started.elapsed();
    let started = std::time::Instant::now();
    let second = snapshot_tree(&f.repo, &lane).unwrap();
    let after = started.elapsed();
    println!("10k-file no-change snapshots: {before:?}, {after:?}");
    assert_eq!(first, second);
}
