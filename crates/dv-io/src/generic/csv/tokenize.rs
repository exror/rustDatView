use dv_core::IoError;

/// A logical record can span physical lines when a quoted field contains a newline.
#[derive(Clone, Copy)]
pub(super) struct Record<'a> {
    pub text: &'a str,
    pub line: usize,
}

pub(super) fn records(text: &str) -> Result<Vec<Record<'_>>, IoError> {
    let bytes = text.as_bytes();
    let mut records = Vec::new();
    let (mut start, mut i, mut line, mut start_line) = (0, 0, 0, 0);
    let mut quoted = false;
    while i < bytes.len() {
        match bytes[i] {
            b'"' if quoted && bytes.get(i + 1) == Some(&b'"') => i += 1,
            b'"' => quoted = !quoted,
            b'\n' => {
                if !quoted {
                    let record = text[start..i].trim_end_matches('\r');
                    if !record.trim().is_empty() {
                        records.push(Record {
                            text: record,
                            line: start_line,
                        });
                    }
                    start = i + 1;
                    start_line = line + 1;
                }
                line += 1;
            }
            _ => {}
        }
        i += 1;
    }
    if quoted {
        return Err(IoError::WrongFormat("unterminated quoted field".into()));
    }
    if start < bytes.len() {
        let record = text[start..].trim_end_matches('\r');
        if !record.trim().is_empty() {
            records.push(Record {
                text: record,
                line: start_line,
            });
        }
    }
    Ok(records)
}

/// Splits fields as slices into the decoded file, without per-cell allocation.
pub(super) struct Fields<'a> {
    text: &'a str,
    separator: u8,
    position: usize,
    finished: bool,
}

impl<'a> Fields<'a> {
    pub fn new(text: &'a str, separator: u8) -> Self {
        Self {
            text,
            separator,
            position: 0,
            finished: false,
        }
    }
}

impl<'a> Iterator for Fields<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        let bytes = self.text.as_bytes();
        let whitespace = self.separator == b' ';
        if whitespace {
            while self.position < bytes.len() && bytes[self.position].is_ascii_whitespace() {
                self.position += 1;
            }
            if self.position == bytes.len() {
                self.finished = true;
                return None;
            }
        }
        let start = self.position;
        let mut quoted = false;
        while self.position < bytes.len() {
            let ch = bytes[self.position];
            if ch == b'"' {
                if quoted && bytes.get(self.position + 1) == Some(&b'"') {
                    self.position += 2;
                    continue;
                }
                quoted = !quoted;
            } else if !quoted
                && if whitespace {
                    ch.is_ascii_whitespace()
                } else {
                    ch == self.separator
                }
            {
                let end = self.position;
                self.position += 1;
                return Some(self.text[start..end].trim());
            }
            self.position += 1;
        }
        self.finished = true;
        Some(self.text[start..].trim())
    }
}

/// Remove CSV quotes; escaped quotes are decoded only for text fields.
pub(super) fn unquote(field: &str) -> &str {
    let field = field.trim();
    if field.len() >= 2 && field.starts_with('"') && field.ends_with('"') {
        field[1..field.len() - 1].trim()
    } else {
        field
    }
}

pub(super) fn owned_field(field: &str) -> String {
    let field = unquote(field);
    if field.contains("\"\"") {
        field.replace("\"\"", "\"")
    } else {
        field.to_owned()
    }
}

pub(super) fn without_comment(text: &str, marker: Option<char>) -> &str {
    let Some(marker) = marker else { return text };
    let mut quoted = false;
    let mut chars = text.char_indices().peekable();
    while let Some((i, ch)) = chars.next() {
        if ch == '"' {
            if quoted && chars.peek().is_some_and(|(_, next)| *next == '"') {
                chars.next();
            } else {
                quoted = !quoted;
            }
        } else if !quoted && ch == marker {
            return text[..i].trim_end();
        }
    }
    text
}
