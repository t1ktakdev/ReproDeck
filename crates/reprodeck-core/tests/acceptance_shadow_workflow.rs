use reprodeck_core::{
    db::init_db,
    repository, shadow_session,
    state_machine::{self, SessionState},
    workflow::{self, ReproductionPhase, SessionMeta},
};
use std::path::Path;
use std::process::Command;
use tempfile::{tempdir, NamedTempFile};

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("git must be installed for acceptance tests");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn before_fix_after_apply_keeps_original_untouched_until_apply() {
    let repo_dir = tempdir().unwrap();
    git(repo_dir.path(), &["init"]);
    git(
        repo_dir.path(),
        &["config", "user.name", "ReproDeck Acceptance"],
    );
    git(
        repo_dir.path(),
        &["config", "user.email", "acceptance@reprodeck.invalid"],
    );
    // Acceptance fixtures must be byte-stable regardless of the tester's
    // global Git line-ending policy (especially core.autocrlf on Windows).
    git(repo_dir.path(), &["config", "core.autocrlf", "false"]);
    std::fs::write(repo_dir.path().join("state.txt"), "BAD\n").unwrap();
    git(repo_dir.path(), &["add", "state.txt"]);
    git(repo_dir.path(), &["commit", "-m", "broken fixture"]);

    let db_file = NamedTempFile::new().unwrap();
    let mut conn = init_db(db_file.path()).unwrap();
    workflow::create_bug_session(
        &conn,
        "acceptance",
        &SessionMeta {
            title: "state check".into(),
            expected: "state is GOOD".into(),
            actual: "state is BAD".into(),
            notes: String::new(),
        },
    )
    .unwrap();
    repository::attach_repository_to_session(&mut conn, "acceptance", repo_dir.path()).unwrap();
    state_machine::transition_session(&conn, "acceptance", SessionState::Preparing).unwrap();
    state_machine::transition_session(&conn, "acceptance", SessionState::CreatingWorkspace)
        .unwrap();
    let shadow = shadow_session::create_session_shadow(&conn, "acceptance").unwrap();
    state_machine::transition_session(&conn, "acceptance", SessionState::Ready).unwrap();

    let step = workflow::add_reproduction_step(
        &conn,
        "acceptance",
        "git",
        &[
            "grep".into(),
            "-q".into(),
            "GOOD".into(),
            "--".into(),
            "state.txt".into(),
        ],
        0,
    )
    .unwrap();
    let artifacts = tempdir().unwrap();

    let before = workflow::execute_reproduction_step(
        &mut conn,
        artifacts.path(),
        &step.id,
        ReproductionPhase::Before,
        true,
    )
    .unwrap();
    assert_eq!(before.run.status, "Failed");
    assert_eq!(
        std::fs::read_to_string(repo_dir.path().join("state.txt")).unwrap(),
        "BAD\n"
    );

    std::fs::write(Path::new(&shadow.worktree_path).join("state.txt"), "GOOD\n").unwrap();
    shadow_session::finalize_session_shadow(&conn, "acceptance").unwrap();
    state_machine::transition_session(&conn, "acceptance", SessionState::Fixing).unwrap();

    let after = workflow::execute_reproduction_step(
        &mut conn,
        artifacts.path(),
        &step.id,
        ReproductionPhase::After,
        true,
    )
    .unwrap();
    assert_eq!(after.run.status, "Passed");
    assert_eq!(
        workflow::outcome_for_session(&conn, "acceptance").unwrap(),
        "VerifiedFix"
    );

    // The signature invariant: all reproduction and fix work happened in the
    // shadow, while the user's original file remains byte-for-byte unchanged.
    assert_eq!(
        std::fs::read_to_string(repo_dir.path().join("state.txt")).unwrap(),
        "BAD\n"
    );

    state_machine::transition_session(&conn, "acceptance", SessionState::Applying).unwrap();
    shadow_session::apply_session_shadow(&conn, "acceptance").unwrap();
    state_machine::transition_session(&conn, "acceptance", SessionState::Applied).unwrap();
    assert_eq!(
        std::fs::read_to_string(repo_dir.path().join("state.txt")).unwrap(),
        "GOOD\n"
    );
}
