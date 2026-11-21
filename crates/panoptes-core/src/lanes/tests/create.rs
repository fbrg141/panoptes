use super::*;

#[test]
fn names_are_slugs() {
    assert_eq!(slug("  Fix the login, please! "), "fix-the-login-please");
    assert_eq!(slug("..."), "lane");
    let long = format!("{} b", "a".repeat(47));
    assert_eq!(slug(&long), "a".repeat(47));
}

#[test]
fn names_skip_folders_and_branches_already_in_use() {
    let f = fixture(None);
    std::fs::create_dir_all(f.root.join("fix-login")).unwrap();
    git(&f.repo, &["branch", "panoptes/fix-login-2"]);
    let new = create(
        &f.root,
        &f.project,
        "fix-login",
        &["fix-login-3".into()],
        "Fix login",
        false,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(new.name, "fix-login-4");
    assert_eq!(new.branch, "panoptes/fix-login-4");
}

#[test]
fn setup_with_lots_of_output_finishes() {
    // Far more than a pipe buffer holds.
    let f = fixture(Some(
        "head -c 1000000 /dev/zero | tr '\\0' x; printf ok > done.txt",
    ));
    let new = create_lane(&f);
    let path = PathBuf::from(&new.worktrees[0].path);
    assert!(path.join("done.txt").exists());
}

#[test]
fn creation_branches_copies_files_and_runs_setup() {
    let f = fixture(Some("printf ready > setup.txt"));
    let new = create_lane(&f);
    let path = PathBuf::from(&new.worktrees[0].path);
    assert_eq!(new.branch, "panoptes/fix-login");
    assert_eq!(path, f.root.join("fix-login/app"));
    assert_eq!(
        std::fs::read_to_string(path.join(".env")).unwrap(),
        "secret\n"
    );
    assert_eq!(
        std::fs::read_to_string(path.join("setup.txt")).unwrap(),
        "ready"
    );
    assert_eq!(git(&path, &["branch", "--show-current"]), new.branch);
    assert_eq!(git(&path, &["status", "--porcelain"]), "?? setup.txt");
}

#[test]
fn failed_setup_removes_the_worktree_and_new_branch() {
    let f = fixture(Some("printf partial > setup.txt; exit 7"));
    let error = create(
        &f.root,
        &f.project,
        "fix-login",
        &[],
        "Fix login",
        false,
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("project setup failed"),
        "{error:#}"
    );
    assert!(!f.root.join("fix-login").exists());
    assert!(!ref_exists(&f.repo, "refs/heads/panoptes/fix-login").unwrap());
}

#[test]
fn main_checkout_is_an_opt_in_without_disk_changes() {
    let f = fixture(Some("exit 99"));
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
    assert_eq!(new.worktrees[0].path, f.project.repo);
    assert!(new.worktrees[0].main_checkout);
    assert_eq!(new.branch, "trunk");
    assert!(!f.root.exists());
}
