use super::Platform;

#[derive(Default)]
pub struct LinuxPlatform;

impl Platform for LinuxPlatform {
    fn open_path(&self, path: &std::path::Path) -> Result<(), crate::error::TranslateError> {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| crate::error::TranslateError::Internal(format!("xdg-open: {e}")))
    }
}
