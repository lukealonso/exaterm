use exaterm::demo::WorkspaceBlueprint;
use exaterm::model::{PresentationMode, SessionLaunch, SessionStatus, WorkspaceState};
use std::process::Command;

#[test]
fn demo_workspace_commands_have_program_and_args() {
    let demo = WorkspaceBlueprint::demo();

    assert_eq!(demo.sessions.len(), 5);
    for session in demo.sessions {
        let argv = session.argv();
        assert!(!argv.is_empty());
        assert!(!argv[0].is_empty());
    }
}

#[test]
fn shell_launch_banner_command_is_spawnable() {
    let launch = SessionLaunch::shell("Smoke", "shell", "hello from smoke");
    let output = Command::new(&launch.program)
        .args(&launch.args)
        .env("PS1", "")
        .output()
        .expect("shell launch should spawn");

    assert!(String::from_utf8_lossy(&output.stdout).contains("hello from smoke"));
}

#[test]
fn workspace_state_tracks_focus_separately_from_selection() {
    let mut state = WorkspaceState::new();
    let a = state.add_session(SessionLaunch::shell("A", "shell", "a"));
    let b = state.add_session(SessionLaunch::blocking_prompt("B", "prompt", "b"));

    state.select_session(b);
    state.set_terminal_focus(Some(a));
    state.mark_spawned(a, 99);
    state.mark_spawned(b, 100);

    assert_eq!(state.selected_session(), Some(b));
    assert_eq!(state.focused_terminal(), Some(a));
    assert_eq!(state.sessions()[0].status, SessionStatus::Waiting);
    assert_eq!(state.sessions()[1].status, SessionStatus::Blocked);
}

#[test]
fn workspace_state_enters_and_exits_focused_terminal_mode() {
    let mut state = WorkspaceState::new();
    let session_id = state.add_session(SessionLaunch::shell("A", "shell", "a"));

    state.enter_focus_mode(session_id);
    assert_eq!(state.presentation_mode(), PresentationMode::Focused(session_id));
    assert_eq!(state.focused_session(), Some(session_id));
    assert_eq!(state.focused_terminal(), Some(session_id));

    state.return_to_battlefield();
    assert_eq!(state.presentation_mode(), PresentationMode::Battlefield);
    assert_eq!(state.focused_session(), None);
    assert_eq!(state.focused_terminal(), None);
    assert_eq!(state.selected_session(), Some(session_id));
}
