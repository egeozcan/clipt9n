//! Safe, testable boundary for selected-text capture and inline paste.

use std::borrow::Cow;
use std::time::Duration;

use crate::error::TranslateError;
use crate::platform::{DestinationIdentity, Platform};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopTarget(Option<DestinationIdentity>);

impl DesktopTarget {
    fn from_identity(identity: Option<DestinationIdentity>) -> Self {
        Self(identity)
    }

    #[cfg(test)]
    pub(crate) fn for_test(process_id: i32, destination_id: u64) -> Self {
        Self(Some(DestinationIdentity::for_test(
            process_id,
            destination_id,
        )))
    }

    #[cfg(test)]
    pub(crate) fn unsupported() -> Self {
        Self(None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionSnapshot {
    pub selected_text: String,
    pub target: DesktopTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteDisposition {
    Pasted,
    TargetChanged,
    Unsupported,
}

pub trait DesktopIo: Send {
    fn capture_selection(
        &mut self,
        copy_delay: Duration,
    ) -> Result<SelectionSnapshot, TranslateError>;

    fn write_clipboard(&mut self, text: &str) -> Result<(), TranslateError>;

    fn paste_if_target_current(
        &mut self,
        target: &DesktopTarget,
    ) -> Result<PasteDisposition, TranslateError>;
}

/// Production desktop adapter. Clipboard handles are opened per operation so
/// app construction remains infallible in headless or pre-login sessions.
#[derive(Default)]
pub struct SystemDesktopIo;

impl DesktopIo for SystemDesktopIo {
    fn capture_selection(
        &mut self,
        copy_delay: Duration,
    ) -> Result<SelectionSnapshot, TranslateError> {
        let mut clipboard = ArboardDesktopClipboard::new()?;
        capture_selection_with(&mut clipboard, &crate::platform::current(), copy_delay)
    }

    fn write_clipboard(&mut self, text: &str) -> Result<(), TranslateError> {
        ArboardDesktopClipboard::new()?.write_text(text)
    }

    fn paste_if_target_current(
        &mut self,
        target: &DesktopTarget,
    ) -> Result<PasteDisposition, TranslateError> {
        // Let the destination observe the clipboard ownership change before
        // sending Paste, then verify focus at the last possible moment.
        std::thread::sleep(Duration::from_millis(20));
        paste_if_target_current_with(&crate::platform::current(), target)
    }
}

#[derive(Debug, Clone)]
enum ClipboardContents {
    Empty,
    Text(String),
    Image {
        width: usize,
        height: usize,
        bytes: Vec<u8>,
    },
}

trait DesktopClipboard {
    fn snapshot(&mut self) -> Result<ClipboardContents, TranslateError>;
    fn read_copied_text(&mut self) -> Result<String, TranslateError>;
    fn restore(&mut self, contents: ClipboardContents) -> Result<(), TranslateError>;
    fn write_text(&mut self, text: &str) -> Result<(), TranslateError>;
}

struct ArboardDesktopClipboard {
    inner: arboard::Clipboard,
}

impl ArboardDesktopClipboard {
    fn new() -> Result<Self, TranslateError> {
        arboard::Clipboard::new()
            .map(|inner| Self { inner })
            .map_err(|error| clipboard_error("opening", error))
    }
}

impl DesktopClipboard for ArboardDesktopClipboard {
    fn snapshot(&mut self) -> Result<ClipboardContents, TranslateError> {
        match self.inner.get_text() {
            Ok(text) => Ok(ClipboardContents::Text(text)),
            Err(arboard::Error::ContentNotAvailable) => match self.inner.get_image() {
                Ok(image) => Ok(ClipboardContents::Image {
                    width: image.width,
                    height: image.height,
                    bytes: image.bytes.into_owned(),
                }),
                Err(arboard::Error::ContentNotAvailable) => Ok(ClipboardContents::Empty),
                Err(error) => Err(clipboard_error("reading image from", error)),
            },
            Err(error) => Err(clipboard_error("reading text from", error)),
        }
    }

    fn read_copied_text(&mut self) -> Result<String, TranslateError> {
        self.inner.get_text().map_err(|error| match error {
            arboard::Error::ContentNotAvailable => TranslateError::EmptyOrNonTextClipboard,
            other => clipboard_error("reading copied text from", other),
        })
    }

    fn restore(&mut self, contents: ClipboardContents) -> Result<(), TranslateError> {
        match contents {
            ClipboardContents::Empty => self
                .inner
                .clear()
                .map_err(|error| clipboard_error("clearing", error)),
            ClipboardContents::Text(text) => self
                .inner
                .set_text(text)
                .map_err(|error| clipboard_error("restoring text to", error)),
            ClipboardContents::Image {
                width,
                height,
                bytes,
            } => self
                .inner
                .set_image(arboard::ImageData {
                    width,
                    height,
                    bytes: Cow::Owned(bytes),
                })
                .map_err(|error| clipboard_error("restoring image to", error)),
        }
    }

    fn write_text(&mut self, text: &str) -> Result<(), TranslateError> {
        self.inner
            .set_text(text)
            .map_err(|error| clipboard_error("writing text to", error))
    }
}

fn clipboard_error(action: &str, error: arboard::Error) -> TranslateError {
    TranslateError::InvalidClipboard(format!("{action} clipboard: {error}"))
}

struct ClipboardRestoreGuard<'a, C: DesktopClipboard> {
    clipboard: &'a mut C,
    original: Option<ClipboardContents>,
}

impl<'a, C: DesktopClipboard> ClipboardRestoreGuard<'a, C> {
    fn new(clipboard: &'a mut C, original: ClipboardContents) -> Self {
        Self {
            clipboard,
            original: Some(original),
        }
    }

    fn restore_now(mut self) -> Result<(), TranslateError> {
        let original = self.original.take().expect("restore guard is armed");
        self.clipboard.restore(original)
    }
}

impl<C: DesktopClipboard> Drop for ClipboardRestoreGuard<'_, C> {
    fn drop(&mut self) {
        let Some(original) = self.original.take() else {
            return;
        };
        if let Err(error) = self.clipboard.restore(original) {
            tracing::warn!(%error, "failed to restore clipboard after selected-text capture");
        }
    }
}

fn selection_probe_text() -> String {
    let mut random = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut random);
    let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("clipt9n-selection-probe-{suffix}")
}

fn capture_selection_with<C: DesktopClipboard, P: Platform>(
    clipboard: &mut C,
    platform: &P,
    copy_delay: Duration,
) -> Result<SelectionSnapshot, TranslateError> {
    // Destination identity must be captured before Copy: Copy itself can
    // trigger focus changes in applications with custom clipboard handling.
    let target = DesktopTarget::from_identity(platform.active_destination_identity());
    let original = clipboard.snapshot()?;
    let before_change_count = platform.clipboard_change_count();
    let restore = ClipboardRestoreGuard::new(clipboard, original);
    // Linux and Windows do not expose a clipboard generation through the
    // current platform seam. Put a unique non-secret probe on the clipboard
    // before Copy so a no-op gesture is distinguishable even when the selected
    // text happens to equal the user's original clipboard. The restore guard
    // puts text, image, or empty contents back on every exit path.
    let probe = if before_change_count.is_none() {
        let probe = selection_probe_text();
        restore.clipboard.write_text(&probe)?;
        Some(probe)
    } else {
        None
    };

    platform.copy_selection_to_clipboard()?;
    std::thread::sleep(copy_delay);
    let selected_text = restore.clipboard.read_copied_text()?;
    let after_change_count = platform.clipboard_change_count();
    let copy_changed = match (before_change_count, after_change_count) {
        (Some(before), Some(after)) => after != before,
        _ => probe.as_deref() != Some(selected_text.as_str()),
    };
    if selected_text.trim().is_empty() || !copy_changed {
        return Err(TranslateError::EmptyOrNonTextClipboard);
    }

    restore.restore_now()?;
    Ok(SelectionSnapshot {
        selected_text,
        target,
    })
}

fn paste_if_target_current_with<P: Platform>(
    platform: &P,
    target: &DesktopTarget,
) -> Result<PasteDisposition, TranslateError> {
    let Some(expected_destination) = target.0.as_ref() else {
        return Ok(PasteDisposition::Unsupported);
    };
    let Some(current_destination) = platform.active_destination_identity() else {
        return Ok(PasteDisposition::Unsupported);
    };
    if &current_destination != expected_destination {
        return Ok(PasteDisposition::TargetChanged);
    }

    platform.paste_from_clipboard()?;
    Ok(PasteDisposition::Pasted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{DestinationIdentity, Platform};
    use std::cell::{Cell, RefCell};
    use std::time::Duration;

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum FakeContents {
        Empty,
        Text(String),
        Image {
            width: usize,
            height: usize,
            bytes: Vec<u8>,
        },
    }

    struct FakeClipboard {
        contents: FakeContents,
        copy_result: Result<String, TranslateError>,
        read_current_contents: bool,
        restore_count: usize,
    }

    impl FakeClipboard {
        fn with_text(text: &str) -> Self {
            Self {
                contents: FakeContents::Text(text.to_string()),
                copy_result: Ok("selected".to_string()),
                read_current_contents: false,
                restore_count: 0,
            }
        }
    }

    impl DesktopClipboard for FakeClipboard {
        fn snapshot(&mut self) -> Result<ClipboardContents, TranslateError> {
            Ok(match &self.contents {
                FakeContents::Empty => ClipboardContents::Empty,
                FakeContents::Text(text) => ClipboardContents::Text(text.clone()),
                FakeContents::Image {
                    width,
                    height,
                    bytes,
                } => ClipboardContents::Image {
                    width: *width,
                    height: *height,
                    bytes: bytes.clone(),
                },
            })
        }

        fn read_copied_text(&mut self) -> Result<String, TranslateError> {
            if self.read_current_contents {
                return match &self.contents {
                    FakeContents::Text(text) => Ok(text.clone()),
                    FakeContents::Empty | FakeContents::Image { .. } => {
                        Err(TranslateError::EmptyOrNonTextClipboard)
                    }
                };
            }
            std::mem::replace(&mut self.copy_result, Ok("selected".to_string()))
        }

        fn restore(&mut self, contents: ClipboardContents) -> Result<(), TranslateError> {
            self.restore_count += 1;
            self.contents = match contents {
                ClipboardContents::Empty => FakeContents::Empty,
                ClipboardContents::Text(text) => FakeContents::Text(text),
                ClipboardContents::Image {
                    width,
                    height,
                    bytes,
                } => FakeContents::Image {
                    width,
                    height,
                    bytes,
                },
            };
            Ok(())
        }

        fn write_text(&mut self, text: &str) -> Result<(), TranslateError> {
            self.contents = FakeContents::Text(text.to_string());
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakePlatform {
        current_destination: RefCell<Option<DestinationIdentity>>,
        paste_count: Cell<usize>,
        change_count: Cell<i64>,
        suppress_copy_change: Cell<bool>,
        generation_available: Cell<bool>,
    }

    impl Platform for FakePlatform {
        fn copy_selection_to_clipboard(&self) -> Result<(), TranslateError> {
            if !self.suppress_copy_change.get() {
                self.change_count.set(self.change_count.get() + 1);
            }
            Ok(())
        }

        fn paste_from_clipboard(&self) -> Result<(), TranslateError> {
            self.paste_count.set(self.paste_count.get() + 1);
            Ok(())
        }

        fn active_destination_identity(&self) -> Option<DestinationIdentity> {
            self.current_destination.borrow().clone()
        }

        fn clipboard_change_count(&self) -> Option<i64> {
            self.generation_available
                .get()
                .then(|| self.change_count.get())
        }
    }

    #[test]
    fn capture_restores_original_clipboard_when_selected_text_is_empty() {
        let mut clipboard = FakeClipboard::with_text("saved");
        clipboard.copy_result = Ok(String::new());
        let platform = FakePlatform::default();

        let result = capture_selection_with(&mut clipboard, &platform, Duration::ZERO);

        assert!(matches!(
            result,
            Err(TranslateError::EmptyOrNonTextClipboard)
        ));
        assert_eq!(clipboard.contents, FakeContents::Text("saved".into()));
        assert_eq!(clipboard.restore_count, 1);
    }

    #[test]
    fn capture_restores_initially_empty_clipboard() {
        let mut clipboard = FakeClipboard {
            contents: FakeContents::Empty,
            copy_result: Ok("selected".into()),
            read_current_contents: false,
            restore_count: 0,
        };
        let platform = FakePlatform::default();

        let snapshot = capture_selection_with(&mut clipboard, &platform, Duration::ZERO).unwrap();

        assert_eq!(snapshot.selected_text, "selected");
        assert_eq!(clipboard.contents, FakeContents::Empty);
        assert_eq!(clipboard.restore_count, 1);
    }

    #[test]
    fn capture_restores_supported_non_text_clipboard() {
        let image = FakeContents::Image {
            width: 1,
            height: 1,
            bytes: vec![1, 2, 3, 4],
        };
        let mut clipboard = FakeClipboard {
            contents: image.clone(),
            copy_result: Ok("selected".into()),
            read_current_contents: false,
            restore_count: 0,
        };
        let platform = FakePlatform::default();

        capture_selection_with(&mut clipboard, &platform, Duration::ZERO).unwrap();

        assert_eq!(clipboard.contents, image);
        assert_eq!(clipboard.restore_count, 1);
    }

    #[test]
    fn capture_restores_after_clipboard_read_failure() {
        let mut clipboard = FakeClipboard::with_text("saved");
        clipboard.copy_result = Err(TranslateError::InvalidClipboard("read failed".into()));
        let platform = FakePlatform::default();

        let result = capture_selection_with(&mut clipboard, &platform, Duration::ZERO);

        assert!(matches!(result, Err(TranslateError::InvalidClipboard(_))));
        assert_eq!(clipboard.contents, FakeContents::Text("saved".into()));
        assert_eq!(clipboard.restore_count, 1);
    }

    #[test]
    fn capture_returns_text_and_originating_target() {
        let mut clipboard = FakeClipboard::with_text("saved");
        let platform = FakePlatform {
            current_destination: RefCell::new(Some(DestinationIdentity::for_test(41, 7))),
            ..Default::default()
        };

        let snapshot = capture_selection_with(&mut clipboard, &platform, Duration::ZERO).unwrap();

        assert_eq!(snapshot.selected_text, "selected");
        assert_eq!(snapshot.target, DesktopTarget::for_test(41, 7));
    }

    #[test]
    fn capture_rejects_copy_that_did_not_change_the_clipboard_without_generation_counter() {
        let mut clipboard = FakeClipboard::with_text("saved");
        clipboard.read_current_contents = true;
        let platform = FakePlatform {
            suppress_copy_change: Cell::new(true),
            ..Default::default()
        };

        let result = capture_selection_with(&mut clipboard, &platform, Duration::ZERO);

        assert!(matches!(
            result,
            Err(TranslateError::EmptyOrNonTextClipboard)
        ));
        assert_eq!(clipboard.contents, FakeContents::Text("saved".into()));
    }

    #[test]
    fn capture_accepts_same_text_without_generation_counter_when_copy_succeeds() {
        let mut clipboard = FakeClipboard::with_text("same text");
        clipboard.copy_result = Ok("same text".into());
        let platform = FakePlatform::default();

        let snapshot = capture_selection_with(&mut clipboard, &platform, Duration::ZERO).unwrap();

        assert_eq!(snapshot.selected_text, "same text");
        assert_eq!(clipboard.contents, FakeContents::Text("same text".into()));
    }

    #[test]
    fn paste_is_refused_after_same_process_destination_changes() {
        let platform = FakePlatform {
            current_destination: RefCell::new(Some(DestinationIdentity::for_test(41, 99))),
            ..Default::default()
        };
        let original = DesktopTarget::for_test(41, 7);

        let result = paste_if_target_current_with(&platform, &original).unwrap();

        assert_eq!(result, PasteDisposition::TargetChanged);
        assert_eq!(platform.paste_count.get(), 0);
    }

    #[test]
    fn paste_succeeds_for_same_target() {
        let platform = FakePlatform {
            current_destination: RefCell::new(Some(DestinationIdentity::for_test(41, 7))),
            ..Default::default()
        };
        let original = DesktopTarget::for_test(41, 7);

        let result = paste_if_target_current_with(&platform, &original).unwrap();

        assert_eq!(result, PasteDisposition::Pasted);
        assert_eq!(platform.paste_count.get(), 1);
    }

    #[test]
    fn paste_is_unsupported_without_verifiable_target() {
        let platform = FakePlatform::default();
        let original = DesktopTarget::unsupported();

        let result = paste_if_target_current_with(&platform, &original).unwrap();

        assert_eq!(result, PasteDisposition::Unsupported);
        assert_eq!(platform.paste_count.get(), 0);
    }
}
