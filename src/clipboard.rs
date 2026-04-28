//! Clipboard read/write abstraction.
//!
//! M1 ships `ArboardClipboard` (real impl) and `MockClipboard` (test impl
//! gated to `#[cfg(test)]`). Future milestones add image/file detection beyond
//! the text-only filter and special platform paths via `platform/`.

use crate::error::TranslateError;

pub trait Clipboard: Send + Sync {
    /// Read text from the clipboard. Returns
    /// `TranslateError::EmptyOrNonTextClipboard` if the clipboard is empty,
    /// is not text (e.g. an image), or if the OS denies access.
    fn read_text(&mut self) -> Result<String, TranslateError>;

    /// Write text to the clipboard, replacing whatever is there.
    fn write_text(&mut self, text: &str) -> Result<(), TranslateError>;
}

/// Real clipboard backed by the cross-platform `arboard` crate.
pub struct ArboardClipboard {
    inner: arboard::Clipboard,
}

impl ArboardClipboard {
    pub fn new() -> Result<Self, TranslateError> {
        let inner = arboard::Clipboard::new()
            .map_err(|e| TranslateError::InvalidClipboard(format!("opening clipboard: {e}")))?;
        Ok(Self { inner })
    }
}

impl Clipboard for ArboardClipboard {
    fn read_text(&mut self) -> Result<String, TranslateError> {
        match self.inner.get_text() {
            Ok(s) if s.is_empty() => Err(TranslateError::EmptyOrNonTextClipboard),
            Ok(s) => Ok(s),
            // arboard returns `Error::ContentNotAvailable` for images / files / empty.
            // Treat all as "empty or non-text" for spec §3 UX consistency.
            Err(arboard::Error::ContentNotAvailable) => {
                Err(TranslateError::EmptyOrNonTextClipboard)
            }
            Err(e) => Err(TranslateError::InvalidClipboard(format!(
                "reading clipboard: {e}"
            ))),
        }
    }

    fn write_text(&mut self, text: &str) -> Result<(), TranslateError> {
        self.inner
            .set_text(text)
            .map_err(|e| TranslateError::InvalidClipboard(format!("writing clipboard: {e}")))
    }
}

#[cfg(test)]
pub struct MockClipboard {
    pub read_value: Result<String, TranslateError>,
    pub written: Option<String>,
}

#[cfg(test)]
impl MockClipboard {
    pub fn with_text(text: impl Into<String>) -> Self {
        Self {
            read_value: Ok(text.into()),
            written: None,
        }
    }

    pub fn empty() -> Self {
        Self {
            read_value: Err(TranslateError::EmptyOrNonTextClipboard),
            written: None,
        }
    }
}

#[cfg(test)]
impl Clipboard for MockClipboard {
    fn read_text(&mut self) -> Result<String, TranslateError> {
        match &self.read_value {
            Ok(s) => Ok(s.clone()),
            Err(TranslateError::EmptyOrNonTextClipboard) => {
                Err(TranslateError::EmptyOrNonTextClipboard)
            }
            Err(e) => Err(TranslateError::InvalidClipboard(format!("mock: {e}"))),
        }
    }

    fn write_text(&mut self, text: &str) -> Result<(), TranslateError> {
        self.written = Some(text.to_string());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_with_text_returns_text() {
        let mut c = MockClipboard::with_text("hello");
        assert_eq!(c.read_text().unwrap(), "hello");
    }

    #[test]
    fn mock_empty_returns_empty_or_non_text_error() {
        let mut c = MockClipboard::empty();
        assert!(matches!(
            c.read_text().unwrap_err(),
            TranslateError::EmptyOrNonTextClipboard
        ));
    }

    #[test]
    fn mock_write_records_value() {
        let mut c = MockClipboard::with_text("");
        c.write_text("world").unwrap();
        assert_eq!(c.written, Some("world".to_string()));
    }
}
