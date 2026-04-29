use super::Platform;

#[derive(Default)]
pub struct WindowsPlatform;

impl Platform for WindowsPlatform {
    fn open_path(&self, path: &std::path::Path) -> Result<(), crate::error::TranslateError> {
        std::process::Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| crate::error::TranslateError::Internal(format!("start: {e}")))
    }
}
