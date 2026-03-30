/// Abstraction over platform-specific terminal widgets (VTE4 on GTK, alacritty_terminal on macOS).
pub trait TerminalBackend {
    fn write_output(&mut self, bytes: &[u8]) -> Result<(), String>;
    fn size(&self) -> Option<(u16, u16)>;
    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), String>;
}
