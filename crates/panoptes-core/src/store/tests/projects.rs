use super::*;

#[test]
fn projects_survive_reopening_with_their_settings() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let mut web = settings("web", &[".env", "config/local.toml"]);
    web.setup = Some("pnpm install".into());
    let added = store.add_project("/code/web", &web).unwrap();
    store
        .add_project("/code/api", &settings("api", &[]))
        .unwrap();
    drop(store);

    let store = Store::open(dir.path()).unwrap();
    let projects = store.projects().unwrap();
    assert_eq!(
        projects
            .iter()
            .map(|p| p.settings.name.as_str())
            .collect::<Vec<_>>(),
        ["api", "web"]
    );
    assert_eq!(projects[1], added);
    assert_eq!(projects[1].repo, "/code/web");
    assert_eq!(
        projects[1].settings.copy_files,
        [".env", "config/local.toml"]
    );
    assert_eq!(projects[1].settings.setup.as_deref(), Some("pnpm install"));
    assert!(projects[0].settings.copy_files.is_empty());
}

#[test]
fn a_repository_is_registered_once() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .add_project("/code/web", &settings("web", &[]))
        .unwrap();
    let error = store
        .add_project("/code/web", &settings("again", &[]))
        .unwrap_err()
        .to_string();
    assert!(error.contains("already registered as web"), "{error}");
    assert_eq!(store.projects().unwrap().len(), 1);
}

#[test]
fn updating_a_project_replaces_its_settings() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let id = store
        .add_project("/code/web", &settings("web", &[".env", "a"]))
        .unwrap()
        .id;
    let mut changed = settings("Web app", &["b", ".env"]);
    changed.base_branch = "develop".into();
    changed.setup = Some("make".into());
    let updated = store.update_project(id, &changed).unwrap();
    assert_eq!(updated.repo, "/code/web");
    assert_eq!(store.projects().unwrap(), [updated]);

    store.remove_project(id).unwrap();
    let error = store.update_project(id, &changed).unwrap_err().to_string();
    assert!(error.contains("no longer registered"), "{error}");
}

#[test]
fn removing_a_project_drops_its_files_and_never_reuses_its_id() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let first = store
        .add_project("/code/web", &settings("web", &[".env"]))
        .unwrap();
    store.remove_project(first.id).unwrap();
    assert!(store.projects().unwrap().is_empty());
    assert_eq!(count(&store, "project_copy_files"), 0);

    let second = store
        .add_project("/code/web", &settings("web", &[".env"]))
        .unwrap();
    assert!(second.id > first.id);
}
