//! UTF-8 safe gap buffer with selection, navigation, and bounded multi-level undo/redo.

const DEFAULT_GAP: usize = 128;
const MAX_UNDO_OPS: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
enum EditKind {
    Insert { pos: usize, text: String },
    Delete { pos: usize, text: String },
}

#[derive(Debug, Clone)]
pub struct GapBuffer {
    buf: Vec<u8>,
    gap_start: usize,
    gap_end: usize,
    cursor: usize,
    anchor: usize,
    undo_stack: Vec<EditKind>,
    redo_stack: Vec<EditKind>,
    dirty: bool,
    version: u64,
}

impl Default for GapBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl GapBuffer {
    pub fn new() -> Self {
        Self {
            buf: vec![0u8; DEFAULT_GAP],
            gap_start: 0,
            gap_end: DEFAULT_GAP,
            cursor: 0,
            anchor: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            dirty: false,
            version: 0,
        }
    }

    pub fn from_text(text: &str) -> Self {
        let bytes = text.as_bytes();
        let mut buf = Vec::with_capacity(bytes.len() + DEFAULT_GAP);
        buf.extend_from_slice(bytes);
        buf.resize(bytes.len() + DEFAULT_GAP, 0);
        Self {
            buf,
            gap_start: bytes.len(),
            gap_end: bytes.len() + DEFAULT_GAP,
            cursor: 0,
            anchor: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            dirty: false,
            version: 0,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.buf.len() - (self.gap_end - self.gap_start)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    #[inline]
    pub fn anchor(&self) -> usize {
        self.anchor
    }

    #[inline]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    #[inline]
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    #[inline]
    pub fn version(&self) -> u64 {
        self.version
    }

    #[inline]
    pub fn selection(&self) -> Option<(usize, usize)> {
        if self.anchor == self.cursor {
            None
        } else if self.anchor < self.cursor {
            Some((self.anchor, self.cursor))
        } else {
            Some((self.cursor, self.anchor))
        }
    }

    pub fn clear_selection(&mut self) {
        self.anchor = self.cursor;
    }

    pub fn set_cursor(&mut self, pos: usize, extend_selection: bool) {
        let pos = self.clamp_char_boundary(pos);
        self.cursor = pos;
        if !extend_selection {
            self.anchor = pos;
        }
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.len();
    }

    pub fn select_range(&mut self, start: usize, end: usize) {
        self.anchor = self.clamp_char_boundary(start);
        self.cursor = self.clamp_char_boundary(end);
    }

    #[inline]
    pub fn byte_at(&self, pos: usize) -> Option<u8> {
        let len = self.len();
        if pos >= len {
            return None;
        }
        if pos < self.gap_start {
            Some(self.buf[pos])
        } else {
            Some(self.buf[pos + (self.gap_end - self.gap_start)])
        }
    }

    #[inline]
    pub fn is_char_boundary(&self, pos: usize) -> bool {
        let len = self.len();
        if pos == 0 || pos == len {
            return true;
        }
        if pos > len {
            return false;
        }
        match self.byte_at(pos) {
            Some(b) => (b as i8) >= -0x40,
            None => false,
        }
    }

    pub fn clamp_char_boundary(&self, mut pos: usize) -> usize {
        let len = self.len();
        if pos > len {
            pos = len;
        }
        while pos > 0 && !self.is_char_boundary(pos) {
            pos -= 1;
        }
        pos
    }

    pub fn next_char_boundary(&self, pos: usize) -> usize {
        let len = self.len();
        if pos >= len {
            return len;
        }
        let mut p = pos + 1;
        while p < len && !self.is_char_boundary(p) {
            p += 1;
        }
        p
    }

    pub fn prev_char_boundary(&self, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }
        let mut p = pos.min(self.len()) - 1;
        while p > 0 && !self.is_char_boundary(p) {
            p -= 1;
        }
        p
    }

    fn move_gap(&mut self, pos: usize) {
        let pos = pos.min(self.len());
        if pos == self.gap_start {
            return;
        }
        let gap_len = self.gap_end - self.gap_start;
        if pos < self.gap_start {
            let delta = self.gap_start - pos;
            self.buf.copy_within(pos..self.gap_start, self.gap_end - delta);
            self.gap_start = pos;
            self.gap_end -= delta;
        } else {
            let delta = pos - self.gap_start;
            self.buf
                .copy_within(self.gap_end..self.gap_end + delta, self.gap_start);
            self.gap_start += delta;
            self.gap_end += delta;
        }
        debug_assert_eq!(self.gap_end - self.gap_start, gap_len);
    }

    fn ensure_gap(&mut self, needed: usize) {
        let cur_gap = self.gap_end - self.gap_start;
        if cur_gap >= needed {
            return;
        }
        let grow = (needed - cur_gap).max(DEFAULT_GAP).max(self.len() / 4);
        let old_len = self.buf.len();
        let after_len = old_len - self.gap_end;
        self.buf.resize(old_len + grow, 0);
        self.buf
            .copy_within(self.gap_end..old_len, self.gap_end + grow);
        self.gap_end += grow;
        debug_assert_eq!(self.buf.len() - self.gap_end, after_len);
    }

    fn raw_insert(&mut self, pos: usize, text: &str) {
        if text.is_empty() {
            return;
        }
        let bytes = text.as_bytes();
        self.move_gap(pos);
        self.ensure_gap(bytes.len());
        self.buf[self.gap_start..self.gap_start + bytes.len()].copy_from_slice(bytes);
        self.gap_start += bytes.len();
        self.dirty = true;
        self.version = self.version.wrapping_add(1);
    }

    fn raw_delete(&mut self, start: usize, end: usize) -> String {
        if start >= end {
            return String::new();
        }
        let end = end.min(self.len());
        self.move_gap(start);
        let count = end - start;
        let removed =
            String::from_utf8_lossy(&self.buf[self.gap_end..self.gap_end + count]).into_owned();
        self.gap_end += count;
        self.dirty = true;
        self.version = self.version.wrapping_add(1);
        removed
    }

    fn push_undo(&mut self, op: EditKind) {
        // Coalesce consecutive single-char inserts or backspaces for natural undo
        if let Some(last) = self.undo_stack.last_mut() {
            match (last, &op) {
                (
                    EditKind::Insert {
                        pos: lpos,
                        text: ltext,
                    },
                    EditKind::Insert { pos, text },
                ) if *pos == *lpos + ltext.len()
                    && text.len() <= 4
                    && !text.contains('\n')
                    && ltext.len() < 64 =>
                {
                    ltext.push_str(text);
                    return;
                }
                (
                    EditKind::Delete {
                        pos: lpos,
                        text: ltext,
                    },
                    EditKind::Delete { pos, text },
                ) if *pos + text.len() == *lpos
                    && text.len() <= 4
                    && !text.contains('\n')
                    && ltext.len() < 64 =>
                {
                    let mut combined = text.clone();
                    combined.push_str(ltext);
                    *lpos = *pos;
                    *ltext = combined;
                    return;
                }
                _ => {}
            }
        }
        if self.undo_stack.len() >= MAX_UNDO_OPS {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(op);
    }

    pub fn insert_str(&mut self, text: &str) {
        if let Some((s, e)) = self.selection() {
            self.delete_range(s, e);
        }
        if text.is_empty() {
            return;
        }
        let pos = self.cursor;
        self.raw_insert(pos, text);
        self.cursor = pos + text.len();
        self.anchor = self.cursor;
        self.redo_stack.clear();
        self.push_undo(EditKind::Insert {
            pos,
            text: text.to_string(),
        });
    }

    pub fn delete_range(&mut self, start: usize, end: usize) -> String {
        let start = self.clamp_char_boundary(start);
        let end = self.clamp_char_boundary(end);
        if start >= end {
            return String::new();
        }
        let removed = self.raw_delete(start, end);
        self.cursor = start;
        self.anchor = start;
        self.redo_stack.clear();
        self.push_undo(EditKind::Delete {
            pos: start,
            text: removed.clone(),
        });
        removed
    }

    pub fn delete_selection(&mut self) -> Option<String> {
        let (s, e) = self.selection()?;
        Some(self.delete_range(s, e))
    }

    pub fn backspace(&mut self) {
        if self.delete_selection().is_some() {
            return;
        }
        if self.cursor == 0 {
            return;
        }
        let prev = self.prev_char_boundary(self.cursor);
        let cur = self.cursor;
        self.delete_range(prev, cur);
    }

    pub fn backspace_word(&mut self) {
        if self.delete_selection().is_some() {
            return;
        }
        let start = self.prev_word_boundary(self.cursor);
        let end = self.cursor;
        self.delete_range(start, end);
    }

    pub fn delete_forward(&mut self) {
        if self.delete_selection().is_some() {
            return;
        }
        if self.cursor >= self.len() {
            return;
        }
        let next = self.next_char_boundary(self.cursor);
        let cur = self.cursor;
        self.delete_range(cur, next);
    }

    pub fn delete_word_forward(&mut self) {
        if self.delete_selection().is_some() {
            return;
        }
        let start = self.cursor;
        let end = self.next_word_boundary(self.cursor);
        self.delete_range(start, end);
    }

    pub fn undo(&mut self) -> bool {
        let Some(op) = self.undo_stack.pop() else {
            return false;
        };
        match op {
            EditKind::Insert { pos, text } => {
                self.raw_delete(pos, pos + text.len());
                self.cursor = pos;
                self.anchor = pos;
                self.redo_stack.push(EditKind::Insert { pos, text });
            }
            EditKind::Delete { pos, text } => {
                self.raw_insert(pos, &text);
                self.cursor = pos + text.len();
                self.anchor = self.cursor;
                self.redo_stack.push(EditKind::Delete { pos, text });
            }
        }
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(op) = self.redo_stack.pop() else {
            return false;
        };
        match op {
            EditKind::Insert { pos, text } => {
                self.raw_insert(pos, &text);
                self.cursor = pos + text.len();
                self.anchor = self.cursor;
                self.undo_stack.push(EditKind::Insert { pos, text });
            }
            EditKind::Delete { pos, text } => {
                self.raw_delete(pos, pos + text.len());
                self.cursor = pos;
                self.anchor = pos;
                self.undo_stack.push(EditKind::Delete { pos, text });
            }
        }
        true
    }

    pub fn to_string_lossy(&self) -> String {
        let mut out = Vec::with_capacity(self.len());
        out.extend_from_slice(&self.buf[..self.gap_start]);
        out.extend_from_slice(&self.buf[self.gap_end..]);
        // Buffer only ever receives valid UTF-8, so from_utf8 succeeds without reallocation
        String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
    }

    pub fn slice_string(&self, start: usize, end: usize) -> String {
        let start = self.clamp_char_boundary(start);
        let end = self.clamp_char_boundary(end);
        if start >= end {
            return String::new();
        }
        let mut out = Vec::with_capacity(end - start);
        if end <= self.gap_start {
            out.extend_from_slice(&self.buf[start..end]);
        } else if start >= self.gap_start {
            let gap = self.gap_end - self.gap_start;
            out.extend_from_slice(&self.buf[start + gap..end + gap]);
        } else {
            let gap = self.gap_end - self.gap_start;
            out.extend_from_slice(&self.buf[start..self.gap_start]);
            out.extend_from_slice(&self.buf[self.gap_end..end + gap]);
        }
        String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
    }

    pub fn selected_text(&self) -> Option<String> {
        let (s, e) = self.selection()?;
        Some(self.slice_string(s, e))
    }

    pub fn replace_all_text(&mut self, text: &str) {
        let len = self.len();
        if len > 0 {
            self.delete_range(0, len);
        }
        if !text.is_empty() {
            self.insert_str(text);
        }
    }

    pub fn line_start(&self, pos: usize) -> usize {
        let mut p = pos.min(self.len());
        while p > 0 {
            if self.byte_at(p - 1) == Some(b'\n') {
                break;
            }
            p -= 1;
        }
        p
    }

    pub fn line_end(&self, pos: usize) -> usize {
        let len = self.len();
        let mut p = pos.min(len);
        while p < len {
            if self.byte_at(p) == Some(b'\n') {
                break;
            }
            p += 1;
        }
        p
    }

    pub fn move_left(&mut self, extend: bool) {
        if !extend {
            if let Some((s, _)) = self.selection() {
                self.cursor = s;
                self.anchor = s;
                return;
            }
        }
        let next = self.prev_char_boundary(self.cursor);
        self.set_cursor(next, extend);
    }

    pub fn move_right(&mut self, extend: bool) {
        if !extend {
            if let Some((_, e)) = self.selection() {
                self.cursor = e;
                self.anchor = e;
                return;
            }
        }
        let next = self.next_char_boundary(self.cursor);
        self.set_cursor(next, extend);
    }

    fn is_word_byte(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
    }

    pub fn prev_word_boundary(&self, pos: usize) -> usize {
        let mut p = pos.min(self.len());
        while p > 0 {
            let prev = self.prev_char_boundary(p);
            if self.byte_at(prev).map(Self::is_word_byte).unwrap_or(false) {
                break;
            }
            p = prev;
        }
        while p > 0 {
            let prev = self.prev_char_boundary(p);
            if !self.byte_at(prev).map(Self::is_word_byte).unwrap_or(false) {
                break;
            }
            p = prev;
        }
        p
    }

    pub fn next_word_boundary(&self, pos: usize) -> usize {
        let len = self.len();
        let mut p = pos.min(len);
        while p < len && self.byte_at(p).map(Self::is_word_byte).unwrap_or(false) {
            p = self.next_char_boundary(p);
        }
        while p < len && !self.byte_at(p).map(Self::is_word_byte).unwrap_or(false) {
            p = self.next_char_boundary(p);
        }
        p
    }

    pub fn move_word_left(&mut self, extend: bool) {
        let p = self.prev_word_boundary(self.cursor);
        self.set_cursor(p, extend);
    }

    pub fn move_word_right(&mut self, extend: bool) {
        let p = self.next_word_boundary(self.cursor);
        self.set_cursor(p, extend);
    }

    pub fn move_line_start(&mut self, extend: bool) {
        let p = self.line_start(self.cursor);
        self.set_cursor(p, extend);
    }

    pub fn move_line_end(&mut self, extend: bool) {
        let p = self.line_end(self.cursor);
        self.set_cursor(p, extend);
    }

    pub fn move_up(&mut self, extend: bool) {
        let cur_start = self.line_start(self.cursor);
        if cur_start == 0 {
            self.set_cursor(0, extend);
            return;
        }
        let col = self.cursor - cur_start;
        let prev_end = cur_start - 1;
        let prev_start = self.line_start(prev_end);
        let target = (prev_start + col).min(prev_end);
        self.set_cursor(target, extend);
    }

    pub fn move_down(&mut self, extend: bool) {
        let cur_start = self.line_start(self.cursor);
        let cur_end = self.line_end(self.cursor);
        if cur_end >= self.len() {
            self.set_cursor(self.len(), extend);
            return;
        }
        let col = self.cursor - cur_start;
        let next_start = cur_end + 1;
        let next_end = self.line_end(next_start);
        let target = (next_start + col).min(next_end);
        self.set_cursor(target, extend);
    }

    pub fn first_non_empty_line(&self) -> String {
        let text = self.to_string_lossy();
        for line in text.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                let mut s: String = trimmed.chars().take(80).collect();
                s = s.replace('\t', " ");
                return s;
            }
        }
        String::from("(empty note)")
    }

    /// Toggles markdown checkbox `- [ ] ` <-> `- [x] ` on the cursor's current line.
    pub fn toggle_checkbox_at_cursor(&mut self) -> bool {
        let start = self.line_start(self.cursor);
        let end = self.line_end(self.cursor);
        let line = self.slice_string(start, end);
        let leading_ws = line.len() - line.trim_start_matches([' ', '\t']).len();
        let rest = &line[leading_ws..];
        let replacement = if rest.starts_with("- [ ] ") {
            Some((start + leading_ws, start + leading_ws + 6, "- [x] "))
        } else if rest.starts_with("- [x] ") || rest.starts_with("- [X] ") {
            Some((start + leading_ws, start + leading_ws + 6, "- [ ] "))
        } else {
            None
        };
        if let Some((s, e, rep)) = replacement {
            let old_cursor = self.cursor;
            self.delete_range(s, e);
            self.set_cursor(s, false);
            self.insert_str(rep);
            self.set_cursor(old_cursor, false);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_delete_undo_redo() {
        let mut gb = GapBuffer::new();
        gb.insert_str("Hello");
        gb.insert_str(" world");
        assert_eq!(gb.to_string_lossy(), "Hello world");
        assert!(gb.is_dirty());

        gb.set_cursor(5, false);
        gb.insert_str(" Kadex");
        assert_eq!(gb.to_string_lossy(), "Hello Kadex world");

        assert!(gb.undo());
        assert_eq!(gb.to_string_lossy(), "Hello world");
        assert!(gb.redo());
        assert_eq!(gb.to_string_lossy(), "Hello Kadex world");
    }

    #[test]
    fn unicode_and_navigation() {
        let mut gb = GapBuffer::from_text("héllo 🌍\nsecond line");
        gb.set_cursor(0, false);
        gb.move_right(false);
        gb.move_right(false); // past 'h' and 'é'
        assert_eq!(gb.slice_string(0, gb.cursor()), "hé");
        gb.move_down(false);
        assert!(gb.cursor() > 8);
        gb.move_line_start(false);
        assert_eq!(gb.slice_string(gb.cursor(), gb.len()), "second line");
    }

    #[test]
    fn checkbox_toggle() {
        let mut gb = GapBuffer::from_text("  - [ ] buy milk");
        gb.set_cursor(4, false);
        assert!(gb.toggle_checkbox_at_cursor());
        assert_eq!(gb.to_string_lossy(), "  - [x] buy milk");
        assert!(gb.toggle_checkbox_at_cursor());
        assert_eq!(gb.to_string_lossy(), "  - [ ] buy milk");
    }
}
