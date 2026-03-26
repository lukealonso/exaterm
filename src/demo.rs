use crate::model::SessionLaunch;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceBlueprint {
    pub name: String,
    pub sessions: Vec<SessionLaunch>,
}

impl WorkspaceBlueprint {
    pub fn demo() -> Self {
        let planner_dir = prepare_demo_dir(
            "planner",
            &[("notes/plan.txt", "Investigate parser regressions\n")],
        );
        let worker_dir = prepare_demo_dir(
            "pulse-stream",
            &[
                ("src/parser.rs", "// parser workbench\n"),
                ("tests/parser.rs", "// parser tests\n"),
            ],
        );
        let approval_dir = prepare_demo_dir("approval-gate", &[("drafts/request.txt", "needs approval\n")]);
        let failed_dir = prepare_demo_dir("failed-task", &[("logs/build.log", "missing dependency graph edge\n")]);
        let shell_dir = prepare_demo_dir("operator-shell", &[("README.txt", "interactive shell\n")]);

        Self {
            name: "Built-in Demo Workspace".into(),
            sessions: vec![
                SessionLaunch::planning_stream(
                    "Planner",
                    "Thinking stream",
                    "messages=(\
                        'Investigating parser regressions before touching code.' \
                        'Comparing the last failing tests with the config loader.' \
                        'Need to confirm whether the parser issue comes from config hydration.'\
                    ); \
                    i=0; \
                    while true; do \
                        printf '%s\\r\\n' \"${messages[$((i % 3))]}\"; \
                        i=$((i+1)); \
                        sleep 4; \
                    done",
                )
                .with_cwd(planner_dir),
                SessionLaunch::running_stream(
                    "Pulse Stream",
                    "Working tree",
                    "i=1; \
                    while true; do \
                        printf '[%s] cargo test parser: %d failures remain\\r\\n' \"$(date +%T)\" \"$((4 - (i % 3)))\"; \
                        printf '// pass %03d\\n' \"$i\" >> src/parser.rs; \
                        printf '// assertion %03d\\n' \"$i\" >> tests/parser.rs; \
                        i=$((i+1)); \
                        sleep 3; \
                    done",
                )
                .with_cwd(worker_dir),
                SessionLaunch::blocking_prompt(
                    "Approval Gate",
                    "Blocked prompt",
                    "Waiting for approval. Type a response and press Enter.",
                )
                .with_cwd(approval_dir),
                SessionLaunch::failing_task(
                    "Failed Task",
                    "Failure signal",
                    "Compilation failed: missing dependency graph edge.",
                    2,
                )
                .with_cwd(failed_dir),
                SessionLaunch::shell(
                    "Operator Shell",
                    "Waiting shell",
                    "Operator shell is ready for direct intervention.",
                )
                .with_cwd(shell_dir),
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

fn prepare_demo_dir(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = std::env::temp_dir().join("exaterm-demo").join(name);
    fs::create_dir_all(&root).expect("demo directory should exist");
    for (relative, contents) in files {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("demo subdirectory should exist");
        }
        fs::write(path, contents).expect("demo seed file should be writable");
    }
    root
}

#[cfg(test)]
mod tests {
    use super::WorkspaceBlueprint;
    use crate::model::SessionKind;

    #[test]
    fn demo_workspace_has_expected_shape() {
        let workspace = WorkspaceBlueprint::demo();

        assert_eq!(workspace.name, "Built-in Demo Workspace");
        assert_eq!(workspace.sessions.len(), 5);
        assert!(workspace
            .sessions
            .iter()
            .any(|session| session.name == "Approval Gate"));
        assert!(workspace
            .sessions
            .iter()
            .any(|session| session.kind == SessionKind::FailingTask));
        assert!(workspace
            .sessions
            .iter()
            .any(|session| session.kind == SessionKind::PlanningStream));
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
