use std::borrow::Cow;

use super::MAX_SSE_EVENT_DATA_BYTES;

/// Incremental SSE event parser. Transport chunks are split into logical lines
/// by the caller; this type handles field semantics and multiline `data` values.
#[derive(Default)]
pub(super) struct SseEventParser {
    pub(super) data: String,
    spare: String,
}

impl SseEventParser {
    pub(super) fn push_line(&mut self, line: &str) -> Result<Option<String>, ()> {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let line = line.strip_prefix('\u{feff}').unwrap_or(line);
        if line.is_empty() {
            return Ok(self.dispatch());
        }
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.strip_prefix(' ').unwrap_or(data);
            let added = data.len() + usize::from(!self.data.is_empty());
            if self.data.len().saturating_add(added) > MAX_SSE_EVENT_DATA_BYTES {
                return Err(());
            }
            if !self.data.is_empty() {
                self.data.push('\n');
            }
            self.data.push_str(data);
        }
        Ok(None)
    }

    pub(super) fn finish(&mut self) -> Option<String> {
        self.dispatch()
    }

    fn dispatch(&mut self) -> Option<String> {
        if self.data.is_empty() {
            return None;
        }
        std::mem::swap(&mut self.data, &mut self.spare);
        Some(std::mem::take(&mut self.spare))
    }

    pub(super) fn recycle(&mut self, mut data: String) {
        data.clear();
        if data.capacity() > self.data.capacity() {
            self.data = data;
        }
    }
}

pub(super) fn utf8_lossy_slice(bytes: &[u8]) -> Cow<'_, str> {
    match std::str::from_utf8(bytes) {
        Ok(line) => Cow::Borrowed(line),
        Err(_) => Cow::Owned(String::from_utf8_lossy(bytes).into_owned()),
    }
}
