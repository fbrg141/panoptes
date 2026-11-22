use super::*;

fn version(dir: &Path) -> u32 {
    let conn = Connection::open(dir.join(DATABASE)).unwrap();
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap()
}

fn step(apply: fn(&Transaction, &Path) -> anyhow::Result<()>) -> Migration {
    Migration {
        apply,
        committed: |_| {},
    }
}

fn broken(_: &Transaction, _: &Path) -> anyhow::Result<()> {
    bail!("no room")
}

fn v2_notes(tx: &Transaction, _: &Path) -> anyhow::Result<()> {
    tx.execute_batch("CREATE TABLE notes (body TEXT NOT NULL) STRICT;")?;
    Ok(())
}

fn v2_parents(tx: &Transaction, _: &Path) -> anyhow::Result<()> {
    tx.execute_batch(
        "CREATE TABLE parents (id INTEGER PRIMARY KEY) STRICT;
        CREATE TABLE children (
            parent INTEGER NOT NULL REFERENCES parents (id) ON DELETE CASCADE
        ) STRICT;
        INSERT INTO parents (id) VALUES (1);
        INSERT INTO children (parent) VALUES (1);",
    )?;
    Ok(())
}

/// SQLite's way to change a table: build a new one, copy, drop, rename.
fn v3_rebuild_parents(tx: &Transaction, _: &Path) -> anyhow::Result<()> {
    tx.execute_batch(
        "CREATE TABLE parents_new (id INTEGER PRIMARY KEY, name TEXT) STRICT;
        INSERT INTO parents_new (id) SELECT id FROM parents;
        DROP TABLE parents;
        ALTER TABLE parents_new RENAME TO parents;",
    )?;
    Ok(())
}

fn v3_orphan_child(tx: &Transaction, _: &Path) -> anyhow::Result<()> {
    tx.execute_batch("INSERT INTO children (parent) VALUES (9);")?;
    Ok(())
}

/// The tests of the migration machinery build on a store at schema 1,
/// so they keep their meaning as real steps are added.
fn v1() -> Migration {
    MIGRATIONS[0]
}

#[test]
fn a_new_store_starts_at_the_latest_schema() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert!(store.pinned().unwrap().is_empty());
    assert_eq!(version(dir.path()), MIGRATIONS.len() as u32);
}

#[test]
fn pins_in_index_json_are_imported_once() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join(LEGACY_INDEX);
    std::fs::write(&legacy, r#"{"pinned":["pi:a","pi:b"]}"#).unwrap();

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(
        store.pinned().unwrap(),
        BTreeSet::from(["pi:a".into(), "pi:b".into()])
    );
    assert!(!legacy.exists());
    assert!(dir.path().join("index.json.migrated").exists());

    // Unpinning sticks, and a stray index.json is not imported again.
    store.set_pinned("pi:a", false).unwrap();
    drop(store);
    std::fs::write(&legacy, r#"{"pinned":["pi:a","pi:c"]}"#).unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.pinned().unwrap(), BTreeSet::from(["pi:b".into()]));
}

#[test]
fn an_unreadable_index_json_stops_the_import() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join(LEGACY_INDEX);
    std::fs::write(&legacy, "{not json").unwrap();

    let error = format!("{:#}", Store::open(dir.path()).err().unwrap());
    assert!(error.contains("Migration to schema 1 failed"), "{error}");
    assert!(error.contains("its pins were not imported"), "{error}");
    assert_eq!(std::fs::read_to_string(&legacy).unwrap(), "{not json");
    assert_eq!(version(dir.path()), 0);
}

#[test]
fn a_failed_migration_stops_and_keeps_the_data() {
    let dir = tempfile::tempdir().unwrap();
    Store::open_with(dir.path(), &[v1()])
        .unwrap()
        .set_pinned("pi:a", true)
        .unwrap();

    let error = Store::open_with(dir.path(), &[v1(), step(broken)])
        .err()
        .unwrap();
    let error = format!("{error:#}");
    assert!(
        error.contains("Migration to schema 2 failed: no room"),
        "{error}"
    );
    assert_eq!(version(dir.path()), 1);

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.pinned().unwrap(), BTreeSet::from(["pi:a".into()]));
}

#[test]
fn migrating_backs_up_the_database_first() {
    let dir = tempfile::tempdir().unwrap();
    Store::open_with(dir.path(), &[v1()])
        .unwrap()
        .set_pinned("pi:a", true)
        .unwrap();

    let store = Store::open_with(dir.path(), &[v1(), step(v2_notes)]).unwrap();
    assert_eq!(version(dir.path()), 2);
    assert_eq!(store.pinned().unwrap(), BTreeSet::from(["pi:a".into()]));

    let backup = Connection::open(dir.path().join("panoptes.db.v1.bak")).unwrap();
    let backed_up: Vec<String> = backup
        .prepare("SELECT session_key FROM pins")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(backed_up, ["pi:a"]);
    let backup_version: u32 = backup
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(backup_version, 1);
    assert!(!partial_backup(dir.path()).exists());
}

#[test]
fn a_store_from_a_newer_panoptes_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    Store::open_with(dir.path(), &[v1(), step(v2_notes)]).unwrap();
    let error = format!("{:#}", Store::open_with(dir.path(), &[v1()]).err().unwrap());
    assert!(error.contains("newer version of Panoptes"), "{error}");
    assert_eq!(version(dir.path()), 2);
}

#[test]
fn a_damaged_database_is_refused_not_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(DATABASE);
    std::fs::write(&path, b"this is not a database at all, not even close").unwrap();

    let error = format!("{:#}", Store::open(dir.path()).err().unwrap());
    assert!(error.contains("not a readable database"), "{error}");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"this is not a database at all, not even close"
    );
}

#[test]
fn steps_another_panoptes_already_ran_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    // Both read version 0; the other one migrates first.
    let mut late = Connection::open(dir.path().join(DATABASE)).unwrap();
    Store::open_with(dir.path(), &[v1(), step(v2_notes)]).unwrap();

    migrate(&mut late, dir.path(), 0, &[v1(), step(v2_notes)]).unwrap();
    assert_eq!(version(dir.path()), 2);
}

#[test]
fn a_backup_is_kept_only_if_it_holds_the_version_it_is_named_after() {
    let dir = tempfile::tempdir().unwrap();
    Store::open_with(dir.path(), &[v1()])
        .unwrap()
        .set_pinned("pi:a", true)
        .unwrap();
    // Both read version 1; the other one backs up and migrates first.
    let mut late = Connection::open(dir.path().join(DATABASE)).unwrap();
    Store::open_with(dir.path(), &[v1(), step(v2_notes)]).unwrap();

    migrate(&mut late, dir.path(), 1, &[v1(), step(v2_notes)]).unwrap();
    let backup = Connection::open(dir.path().join("panoptes.db.v1.bak")).unwrap();
    assert_eq!(user_version(&backup).unwrap(), 1);
    assert!(!partial_backup(dir.path()).exists());
}

#[test]
fn a_failed_backup_keeps_the_previous_one() {
    let dir = tempfile::tempdir().unwrap();
    Store::open_with(dir.path(), &[v1()]).unwrap();
    let backup = dir.path().join("panoptes.db.v1.bak");
    std::fs::write(&backup, "an earlier backup").unwrap();
    // A folder in the way makes the copy fail.
    std::fs::create_dir(partial_backup(dir.path())).unwrap();

    let error = Store::open_with(dir.path(), &[v1(), step(v2_notes)])
        .err()
        .unwrap();
    assert!(
        format!("{error:#}").contains("Could not back up"),
        "{error:#}"
    );
    assert_eq!(
        std::fs::read_to_string(&backup).unwrap(),
        "an earlier backup"
    );
    assert_eq!(version(dir.path()), 1);
}

#[test]
fn steps_can_rebuild_a_table_without_losing_the_rows_that_point_at_it() {
    let dir = tempfile::tempdir().unwrap();
    let migrations = [v1(), step(v2_parents), step(v3_rebuild_parents)];
    let store = Store::open_with(dir.path(), &migrations).unwrap();

    let children: u32 = store
        .conn
        .query_row("SELECT count(*) FROM children", [], |row| row.get(0))
        .unwrap();
    assert_eq!(children, 1);
    let enforced: bool = store
        .conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert!(enforced);
}

#[test]
fn a_step_that_leaves_a_broken_reference_fails() {
    let dir = tempfile::tempdir().unwrap();
    let migrations = [v1(), step(v2_parents), step(v3_orphan_child)];
    let error = format!(
        "{:#}",
        Store::open_with(dir.path(), &migrations).err().unwrap()
    );
    assert!(
        error.contains("Migration to schema 3 failed: It left a row in children"),
        "{error}"
    );
    assert_eq!(version(dir.path()), 2);
}

#[test]
fn index_json_is_retired_once_its_pins_are_committed() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join(LEGACY_INDEX);
    std::fs::write(&legacy, r#"{"pinned":["pi:a"]}"#).unwrap();

    // A later step failing does not undo the import, so the file goes.
    Store::open_with(dir.path(), &[v1(), step(broken)])
        .err()
        .unwrap();
    assert_eq!(version(dir.path()), 1);
    assert!(!legacy.exists());
    assert!(dir.path().join(IMPORTED_INDEX).exists());
}

#[test]
fn an_emptied_database_is_refused_once_pins_were_imported() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(LEGACY_INDEX), r#"{"pinned":["pi:a"]}"#).unwrap();
    drop(Store::open(dir.path()).unwrap());
    std::fs::write(dir.path().join(DATABASE), "").unwrap();

    let error = format!("{:#}", Store::open(dir.path()).err().unwrap());
    assert!(error.contains("The database is empty"), "{error}");
    assert_eq!(version(dir.path()), 0);
}

#[test]
fn a_store_held_by_another_panoptes_is_reported_as_in_use() {
    let dir = tempfile::tempdir().unwrap();
    drop(Store::open(dir.path()).unwrap());
    let other = Connection::open(dir.path().join(DATABASE)).unwrap();
    other.execute_batch("BEGIN EXCLUSIVE;").unwrap();

    let error = format!("{:#}", Store::open(dir.path()).err().unwrap());
    assert!(error.contains(IN_USE), "{error}");
    assert!(!error.contains("not a readable database"), "{error}");
}

#[test]
fn a_store_from_before_notifications_starts_with_them_on() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open_with(dir.path(), &MIGRATIONS[..9]).unwrap();
        store.dismiss_permission_warning().unwrap();
    }

    let store = Store::open(dir.path()).unwrap();
    let preferences = store.preferences().unwrap();
    assert_eq!(
        preferences.notifications,
        NotificationPreferences::default()
    );
    // The earlier step's own column is untouched.
    assert!(preferences.permission_warning_dismissed);
}

#[test]
fn appearance_defaults_preserve_existing_ui_preferences() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open_with(dir.path(), &MIGRATIONS[..10]).unwrap();
        store.dismiss_permission_warning().unwrap();
        store
            .set_notification(NotificationField::Finished, false)
            .unwrap();
    }

    let store = Store::open(dir.path()).unwrap();
    let preferences = store.preferences().unwrap();
    assert!(preferences.permission_warning_dismissed);
    assert!(!preferences.notifications.finished);
    assert_eq!(preferences.dark_mode, None);
    assert_eq!(preferences.font_size, DEFAULT_FONT_SIZE);
    assert_eq!(version(dir.path()), MIGRATIONS.len() as u32);
}

/// A store as an earlier build left it, with data in every table that
/// build had, comes through the later steps unchanged.
#[test]
fn a_populated_v2_store_keeps_its_projects() {
    let dir = tempfile::tempdir().unwrap();
    let project = {
        let store = Store::open_with(dir.path(), &MIGRATIONS[..2]).unwrap();
        store.set_pinned("pi:a", true).unwrap();
        store
            .add_project("/code/app", &settings("App", &[".env", "local.toml"]))
            .unwrap()
    };

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(version(dir.path()), MIGRATIONS.len() as u32);
    assert_eq!(store.projects().unwrap(), [project]);
    assert_eq!(store.pinned().unwrap(), BTreeSet::from(["pi:a".into()]));
    assert!(dir.path().join("panoptes.db.v2.bak").exists());
}

#[test]
fn a_populated_v3_store_keeps_its_lanes_and_takes_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let (project, active, archived) = {
        let store = Store::open_with(dir.path(), &MIGRATIONS[..3]).unwrap();
        let project = store
            .add_project("/code/app", &settings("App", &[".env"]))
            .unwrap();
        let lane = |name: &str| NewLane {
            project_id: project.id,
            name: name.into(),
            task: format!("Do {name}"),
            branch: format!("panoptes/{name}"),
            worktrees: vec![NewLaneWorktree {
                project_id: project.id,
                path: format!("/lanes/{name}/app"),
                main_checkout: false,
            }],
        };
        let active = store.add_lane(&lane("one")).unwrap();
        let archived = store.add_lane(&lane("two")).unwrap();
        let archived = store
            .archive_lane(archived.id, Some("refs/panoptes/lanes/two/archive"))
            .unwrap();
        (project, active, archived)
    };

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(version(dir.path()), MIGRATIONS.len() as u32);
    assert_eq!(store.projects().unwrap(), [project]);
    assert_eq!(store.lanes().unwrap(), [active.clone(), archived.clone()]);
    assert_eq!(
        archived.worktrees[0].checkpoint_ref.as_deref(),
        Some("refs/panoptes/lanes/two/archive")
    );
    assert!(dir.path().join("panoptes.db.v3.bak").exists());
    // The step that followed works on the migrated rows.
    let session = store.add_session(active.id, "pi").unwrap();
    assert_eq!(session.lane_id, active.id);
    assert!(store.add_session(archived.id, "pi").is_err());
}

#[test]
fn a_v1_store_migrates_to_projects_and_lanes_and_keeps_its_pins() {
    let dir = tempfile::tempdir().unwrap();
    Store::open_with(dir.path(), &[v1()])
        .unwrap()
        .set_pinned("pi:a", true)
        .unwrap();

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(version(dir.path()), MIGRATIONS.len() as u32);
    assert_eq!(store.pinned().unwrap(), BTreeSet::from(["pi:a".into()]));
    assert!(store.projects().unwrap().is_empty());
    assert!(store.lanes().unwrap().is_empty());
    assert!(dir.path().join("panoptes.db.v1.bak").exists());
}
