use crate::model::SessionLaunch;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceBlueprint {
    pub name: String,
    pub sessions: Vec<SessionLaunch>,
}

impl WorkspaceBlueprint {
    pub fn demo() -> Self {
        Self {
            name: "Built-in Demo Workspace".into(),
            sessions: vec![
                SessionLaunch::shell(
                    "Planner",
                    "Waiting shell",
                    "Planner ready. Type directly in this terminal when intervention is needed.",
                ),
                SessionLaunch::running_stream(
                    "Pulse Stream",
                    "Running task",
                    "i=1; while true; do printf '[%s] heartbeat %03d\\r\\n' \"$(date +%T)\" \"$i\"; i=$((i+1)); sleep 2; done",
                ),
                SessionLaunch::blocking_prompt(
                    "Approval Gate",
                    "Blocked prompt",
                    "Waiting for approval. Type a response and press Enter.",
                ),
                SessionLaunch::failing_task(
                    "Failed Task",
                    "Failure signal",
                    "Compilation failed: missing dependency graph edge.",
                    2,
                ),
            ],
        }
    }

    pub fn add_shell(number: usize) -> SessionLaunch {
        SessionLaunch::shell(
            format!("Shell {number}"),
            "Generic command session",
            format!("Shell {number} started. This terminal is ready for intervention."),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::WorkspaceBlueprint;
    use crate::model::SessionKind;

    #[test]
    fn demo_workspace_has_expected_shape() {
        let workspace = WorkspaceBlueprint::demo();

        assert_eq!(workspace.name, "Built-in Demo Workspace");
        assert_eq!(workspace.sessions.len(), 4);
        assert!(workspace
            .sessions
            .iter()
            .any(|session| session.name == "Approval Gate"));
        assert!(workspace
            .sessions
            .iter()
            .any(|session| session.kind == SessionKind::FailingTask));
    }

    #[test]
    fn add_shell_uses_generic_command_session_copy() {
        let shell = WorkspaceBlueprint::add_shell(3);

        assert_eq!(shell.name, "Shell 3");
        assert_eq!(shell.subtitle, "Generic command session");
        assert_eq!(shell.program, "/usr/bin/env");
        assert_eq!(shell.args[0], "bash");
    }
}
