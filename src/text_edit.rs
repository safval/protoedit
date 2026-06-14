// double width glyphs (✔)
// highlight
// multi-selection
// multi-cursor
//

#![allow(warnings)]
use std::cell::RefCell;
use std::cmp::Ordering;
use unicode_width::UnicodeWidthChar;

// when scrolling down should stop
#[derive(PartialEq, Default, Debug)]
enum LastPageLines {
    #[default]
    Unspecified, // scroll is not allowed if it will show empty page
    OneBlank,    // scroll stop after first blank line appears
    OneNotBlank, // scroll stop when all lines except one are blank
}

#[derive(Debug)]
pub struct TextConfig {
    pub tab_width: u8,
    pub last_page: LastPageLines,
    pub word_wrap: bool,
}

impl Default for TextConfig {
    fn default() -> Self {
        TextConfig {
            tab_width: 8,
            last_page: LastPageLines::Unspecified,
            word_wrap: false,
        }
    }
}

#[derive(Debug)]
struct TextChange {
    pub at: usize,  // byte index
    pub before: String,
    pub after: String,
}

#[derive(Debug)]
enum TextChangeType {
    Type,
    Delete,
    Cut,
    Paste,
}

#[derive(Debug)]
struct TextChangeSet {
    pub command: TextChangeType,
    pub changes: Vec<TextChange>,
}

// text as lines on the screen, whether word wrapped or not
#[derive(Debug)]
pub struct TextLines {
    pub text: String,
    highlight: Vec<u32>,
    // position in the text where the new line start, and start reason: \n or too long
    pub starts: RefCell<Vec<(usize, bool)>>,
    window_width: usize,
    window_height: usize,
}

#[derive(Debug)]
pub struct TextViewer {
    pub lines: TextLines,
    pub x0: usize,
    pub y0: usize,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct TextSelection {
    pub pos: usize,    // cursor position
    pub len: isize,    // how many symbols selected before or after the cursor
    pub x_pref: usize, // remains unchanged when scroll verticaly
}

#[derive(Debug)]
pub struct TextEditor {
    pub view: TextViewer,
    // cursors (len==0) or selections (len>0)
    // sorted in reverse order of position
    pub selected: Vec<TextSelection>,
    // only this cursor remains if multiselect mode stopped
    pub active_cursor_index: usize,
    history: (Vec<TextChangeSet>, Vec<TextChangeSet>), // undo and redo history
}

impl TextChangeSet {
    pub fn new(command: TextChangeType) -> Self {
        TextChangeSet {
            command,
            changes: vec![],
        }
    }
}

impl TextLines {
    pub fn new(text: String, window_width: usize, window_height: usize) -> Self {
        TextLines {
            text,
            highlight: vec![],
            starts: RefCell::new(vec![]),
            window_width,
            window_height,
        }
    }
    fn update_starts(&self, cfg: &TextConfig, window_width: usize) {
        if self.starts.borrow().is_empty() {
            let starts = calc_line_starts(&self.text, cfg, window_width);
            self.starts.replace(starts);
        }
        assert!(self.starts.borrow().len() > 0);
    }

    // one tuple for each line of the text, contains:
    // 0: line start position,
    // 1: line end position,
    // 2: line end type (false if '\n' or true if line length too long)
    pub fn starts_and_ends(&self, cfg: &TextConfig) -> Vec<(usize, usize, bool)> {
        let mut res: Vec<(usize, usize, bool)> = vec![];
        let mut prv = 0;
        self.update_starts(cfg, self.window_width);
        if let Ok(s) = self.starts.try_borrow() {
            res = s.iter().skip(1).map(|item| {
                let r = (prv, (*item).0 - 1, (*item).1);
                prv = (*item).0;
                r
            }
            ).collect();
            res.push((prv, self.text.len(), false));
        } else { debug_assert!(false) }
        res
    }

    pub fn height(&self, cfg: &TextConfig) -> usize {
        self.update_starts(cfg, self.window_width);
        self.starts.borrow().len()
    }

    // if pos >= text.len(), x in position after last char of last line
    pub fn to2d(&self, cfg: &TextConfig, pos: usize) -> (usize, usize) {
        self.update_starts(cfg, self.window_width);
        let starts = self.starts.borrow();

        let starts2: Vec<usize> = starts.iter().map(|item| { item.0 }).collect(); // TODO

        let mut row = match starts2.binary_search(&pos) {
            Ok(y) => y,
            Err(y) => y - 1,
        };
        if row > 0 && row >= starts.len() {
            row -= 1;
        }

        let next_line_start = *starts.get(row + 1).unwrap_or(&(self.text.len(), false));
        let max_col = next_line_start.0 - starts[row].0;
        let col = (pos - starts[row].0).min(max_col);

        let mut x = 0;

        // `col` is a byte offset within the line; because positions are always on a
        // char boundary, `starts[row].0 + col` is a valid char boundary to slice at.
        for c in self.text[starts[row].0..starts[row].0 + col].chars() {
            x += get_char_width(cfg, c) as usize;
        }

        (x, row)
    }

    // return nearest to (x,y) char index in the text, or 0 if the text is empty
    pub fn to1d(&self, cfg: &TextConfig, pos: (usize, usize)) -> usize {
        self.update_starts(cfg, self.window_width);

        let starts = self.starts.borrow();
        //println!("starts:{:?}", starts);
        let line_start = *starts.get(pos.1).unwrap_or(starts.last().unwrap());
        let scan_end = *starts.get(pos.1 + 1).unwrap_or(&(self.text.len(), false));

        let mut x = 0;
        let mut res = line_start.0;
        if scan_end.0 > line_start.0 {
            // Exclude the final char of the scanned range: for a hard line break it is
            // the trailing '\n', and for any line it keeps the cursor from landing past
            // the last char. Dropping a whole char (not one byte) stays on a boundary.
            let mut end = scan_end.0;
            if let Some(last) = self.text[line_start.0..scan_end.0].chars().next_back() {
                end -= last.len_utf8();
            }
            for c in self.text[line_start.0..end].chars() {
                x += get_char_width(cfg, c) as usize;
                if x > pos.0 {
                    break;
                }
                res += c.len_utf8();
            }
        }
        res
    }

    // byte range [start, end) of the visual line containing `pos`, with any trailing
    // '\n' excluded. Used to keep horizontal cursor movement within a single line.
    pub fn line_bounds(&self, cfg: &TextConfig, pos: usize) -> (usize, usize) {
        self.update_starts(cfg, self.window_width);
        let starts = self.starts.borrow();
        let row = match starts.binary_search_by(|item| item.0.cmp(&pos)) {
            Ok(r) => r,
            Err(r) => r - 1,
        };
        let line_start = starts[row].0;
        let mut line_end = starts.get(row + 1).map_or(self.text.len(), |s| s.0);
        if line_end > line_start && self.text.as_bytes()[line_end - 1] == b'\n' {
            line_end -= 1;
        }
        (line_start, line_end)
    }
}

#[test]
fn text_1dto2d() {
    let cfg = TextConfig::default();
    let tl = TextLines::new("".to_string(), 80, 24);
    assert_eq!(tl.to2d(&cfg, 0), (0, 0));
    assert_eq!(tl.to2d(&cfg, 9), (0, 0));

    let tl = TextLines::new("1".to_string(), 80, 24);
    assert_eq!(tl.to2d(&cfg, 0), (0, 0));
    assert_eq!(tl.to2d(&cfg, 9), (1, 0));

    let tl = TextLines::new("1\t2".to_string(), 80, 24);
    assert_eq!(tl.to2d(&cfg, 0), (0, 0));
    assert_eq!(tl.to2d(&cfg, 1), (1, 0));
    assert_eq!(tl.to2d(&cfg, 2), (1 + cfg.tab_width as usize, 0));
    assert_eq!(tl.to2d(&cfg, 3), (2 + cfg.tab_width as usize, 0));

    let tl = TextLines::new("\n".to_string(), 80, 24);
    assert_eq!(tl.to2d(&cfg, 0), (0, 0));
    assert_eq!(tl.to2d(&cfg, 9), (0, 1));

    let tl = TextLines::new("123\n345\n".to_string(), 80, 24);
    assert_eq!(tl.to2d(&cfg, 0), (0, 0));
    assert_eq!(tl.to2d(&cfg, 4), (0, 1));
    assert_eq!(tl.to2d(&cfg, 8), (0, 2));
    assert_eq!(tl.to2d(&cfg, 9), (0, 2));

    let tl = TextLines::new("1\n12\n123".to_string(), 80, 24);
    assert_eq!(tl.to2d(&cfg, 2), (0, 1));
    assert_eq!(tl.to2d(&cfg, 3), (1, 1));
    assert_eq!(tl.to2d(&cfg, 7), (2, 2));
    assert_eq!(tl.to2d(&cfg, 9), (3, 2));
}

#[test]
fn text_2dto1d() {
    let cfg = TextConfig::default();
    let tl = TextLines::new("1".to_string(), 80, 24);
    assert_eq!(tl.to1d(&cfg, (0, 0)), 0);
    assert_eq!(tl.to1d(&cfg, (9, 0)), 0);
    assert_eq!(tl.to1d(&cfg, (0, 9)), 0);
    assert_eq!(tl.to1d(&cfg, (9, 9)), 0);

    let tl = TextLines::new("1\t2".to_string(), 80, 24);
    assert_eq!(cfg.tab_width, 8);
    assert_eq!(tl.to1d(&cfg, (1, 0)), 1);
    assert_eq!(tl.to1d(&cfg, (3, 0)), 1);
    assert_eq!(tl.to1d(&cfg, (6, 0)), 1);
    assert_eq!(tl.to1d(&cfg, (9, 0)), 2);
    assert_eq!(tl.to1d(&cfg, (99, 0)), 2);

    let tl = TextLines::new("1\n12\n123".to_string(), 80, 24);
    assert_eq!(tl.to1d(&cfg, (1, 0)), 1);
    assert_eq!(tl.to1d(&cfg, (2, 0)), 1);
    assert_eq!(tl.to1d(&cfg, (3, 0)), 1);
    assert_eq!(tl.to1d(&cfg, (9, 0)), 1);
    assert_eq!(tl.to1d(&cfg, (0, 1)), 2);
    assert_eq!(tl.to1d(&cfg, (1, 1)), 3);
    assert_eq!(tl.to1d(&cfg, (2, 1)), 4);
    assert_eq!(tl.to1d(&cfg, (3, 1)), 4);
    assert_eq!(tl.to1d(&cfg, (9, 1)), 4);
    assert_eq!(tl.to1d(&cfg, (0, 2)), 5);
    assert_eq!(tl.to1d(&cfg, (1, 2)), 6);
    assert_eq!(tl.to1d(&cfg, (2, 2)), 7);
    assert_eq!(tl.to1d(&cfg, (3, 2)), 7);
    assert_eq!(tl.to1d(&cfg, (9, 2)), 7);
    assert_eq!(tl.to1d(&cfg, (0, 3)), 5);
    assert_eq!(tl.to1d(&cfg, (1, 3)), 6);
    assert_eq!(tl.to1d(&cfg, (2, 3)), 7);
    assert_eq!(tl.to1d(&cfg, (3, 3)), 7);
}

// how many terminal columns the char occupies on screen
fn get_char_width(cfg: &TextConfig, c: char) -> u8 {
    match c {
        // Tab width is a display/config choice, not an intrinsic property of the char.
        '\t' => cfg.tab_width,
        // UnicodeWidthChar implements Unicode Annex #11 (East Asian Width): 0 for
        // combining/zero-width marks, 1 for normal chars, 2 for wide ones (CJK, many
        // emoji). It returns None for control characters, which we render as a single
        // column to stay consistent with the previous behavior.
        //
        // Caveat: "East Asian Ambiguous" characters (e.g. '✔' U+2714) have no fixed
        // width — it depends on the terminal/font. UnicodeWidthChar::width() resolves
        // them to 1 (the Unicode default for non-CJK context), but some terminals render
        // them as 2 (observed in Windows cmd.exe and Far Manager). There is no portable
        // way to know without asking the terminal itself (print the glyph, then read the
        // cursor column via crossterm's cursor::position()). Use width_cjk() instead if
        // ambiguous chars should be treated as wide.
        _ => UnicodeWidthChar::width(c).unwrap_or(1) as u8,
    }
}


fn calc_line_starts(text: &str, cfg: &TextConfig, window_width: usize) -> Vec<(usize, bool)> {
    let mut x = 0;
    let mut last_space_pos = usize::MAX;
    let mut starts = vec![(0, false)];
    for (index, c) in text.char_indices() {
        //
        // todo: iteration over grapheme clusters
        let char_width = get_char_width(cfg, c) as usize;
        let is_whitespace = c.is_whitespace();

        if is_whitespace {
            last_space_pos = index;
        }

        if c == '\n' {
            starts.push((index + 1, false));
            last_space_pos = usize::MAX;
            x = 0;
        } else if x + char_width > window_width {
            let pos =
                if last_space_pos != usize::MAX {
                    let pos = last_space_pos + if is_whitespace { 0 } else { 1 };
                    last_space_pos = usize::MAX;
                    pos
                } else {
                    index
                };
            starts.push((pos, true));
            x = 0;
        } else {
            x += char_width;
        }
    }
    return starts;
}

#[test]
fn count_text_lines() {
    let cfg = TextConfig {
        //window_width: 3,
        tab_width: 2,
        ..Default::default()
    };

    assert_eq!(calc_line_starts("", &cfg, 3), [(0, false)]);
    assert_eq!(calc_line_starts(" ", &cfg, 3), [(0, false)]);
    assert_eq!(calc_line_starts(" 1", &cfg, 3), [(0, false)]);
    assert_eq!(calc_line_starts("1 ", &cfg, 3), [(0, false)]);
    assert_eq!(calc_line_starts("12", &cfg, 3), [(0, false)]);
    assert_eq!(calc_line_starts("123", &cfg, 3), [(0, false)]);
    assert_eq!(calc_line_starts("1234", &cfg, 3), [(0, false), (3, true)]);
    assert_eq!(calc_line_starts("123\n4", &cfg, 3), [(0, false), (4, false)]);
    assert_eq!(calc_line_starts("1 23", &cfg, 3), [(0, false), (2, true)]);
    assert_eq!(calc_line_starts("1  23", &cfg, 3), [(0, false), (3, true)]);
    assert_eq!(calc_line_starts("1   23", &cfg, 3), [(0, false), (3, true)]);
    assert_eq!(calc_line_starts("1    23", &cfg, 3), [(0, false), (3, true)]);
    assert_eq!(calc_line_starts("123 4", &cfg, 3), [(0, false), (3, true)]);
    assert_eq!(calc_line_starts("123 45", &cfg, 3), [(0, false), (3, true)]);
    assert_eq!(calc_line_starts("123 456", &cfg, 3), [(0, false), (3, true)]);
    assert_eq!(calc_line_starts("11 22", &cfg, 3), [(0, false), (3, true)]);
    assert_eq!(calc_line_starts("11 22 33", &cfg, 3), [(0, false), (3, true), (6, true)]);
    assert_eq!(calc_line_starts("\n", &cfg, 3), [(0, false), (1, false)]);
    assert_eq!(calc_line_starts("\n ", &cfg, 3), [(0, false), (1, false)]);
    assert_eq!(calc_line_starts("\n\n", &cfg, 3), [(0, false), (1, false), (2, false)]);
    assert_eq!(calc_line_starts("12\n34", &cfg, 3), [(0, false), (3, false)]);
    assert_eq!(calc_line_starts("123\n345\n", &cfg, 3), [(0, false), (4, false), (8, false)]);

    assert_eq!('ё'.len_utf8(), 2); // multibyte char
    assert_eq!(calc_line_starts("ё12", &cfg, 3), [(0, false)]);
    assert_eq!(calc_line_starts("ё123", &cfg, 3), [(0, false), (4, true)]);

    assert_eq!('🌏'.len_utf8(), 4); // double width char
    assert_eq!(calc_line_starts("🌏1", &cfg, 3), [(0, false)]);
    assert_eq!(calc_line_starts("🌏12", &cfg, 3), [(0, false), (5, true)]);
    assert_eq!(calc_line_starts("🌏🌏", &cfg, 3), [(0, false), (4, true)]);
    assert_eq!(calc_line_starts("1🌏2🌏", &cfg, 3), [(0, false), (5, true)]);
    assert_eq!(calc_line_starts("🌏1🌏", &cfg, 3), [(0, false), (5, true)]);
    assert_eq!(calc_line_starts("🌏\n1", &cfg, 3), [(0, false), (5, false)]);

    // wide whitespace
    assert_eq!(cfg.tab_width, 2);
    assert_eq!(calc_line_starts("1\t", &cfg, 3), [(0, false)]);
    assert_eq!(calc_line_starts("1\t\t", &cfg, 3), [(0, false), (2, true)]); // wrap to the next line
    assert_eq!(calc_line_starts("1\t2", &cfg, 3), [(0, false), (2, true)]);
    assert_eq!(calc_line_starts("\t\t", &cfg, 3), [(0, false), (1, true)]);
}

//? trait UserInteractor { // TextViewer and TextEditor
//?     fn on_key(self, key: u32) -> impl UserInteractor;
//? }
//?
//?  impl UserInteractor for TextEditor {
//?      fn on_key(self, key: u32) -> impl UserInteractor {
//?          self
//?      }
//?  }

impl TextViewer {
    pub fn new(text: String, window_width: usize, window_height: usize) -> Self {
        TextViewer {
            lines: TextLines::new(text, window_width, window_height),
            x0: 0,
            y0: 0,
        }
    }

    pub fn scroll(&mut self, dy: isize, cfg: &TextConfig) {
        // change self.y0
        if dy < 0 {
            // scroll up
            let dy = -dy as usize;
            self.y0 = if self.y0 > dy { self.y0 - dy } else { 0 }
        } else {
            // scroll down
            let dy = dy as usize;
            let text_height = self.lines.height(cfg);

            if cfg.last_page == LastPageLines::Unspecified {
                //                println!("{:?}\ntext_height={}", self, text_height);
                if self.y0 + dy >= text_height {
                    println!("scroll out");
                } else {
                    self.y0 += dy;
                }
            } else {
                // how many empty lines in the window allowed
                let blank_lines = if cfg.last_page == LastPageLines::OneBlank {
                    1
                } else {
                    self.lines.window_height - 1
                    //cfg.window_height - 1
                };

                let max_y0 = if text_height + blank_lines <= self.lines.window_height {
                    0
                } else {
                    text_height + blank_lines - self.lines.window_height
                };

                self.y0 = (self.y0 + dy).min(max_y0);
            }
        }
    }
}

#[test]
fn scroll_empty() {
    for mode in [
        LastPageLines::Unspecified,
        LastPageLines::OneBlank,
        LastPageLines::OneNotBlank,
    ] {
        let cfg = TextConfig {
            //window_width: 5,
            //window_height: 5,
            last_page: mode,
            ..Default::default()
        };

        let mut view = TextViewer::new(String::new(), 80, 24);
        assert_eq!(view.y0, 0);

        view.scroll(1, &cfg);
        assert_eq!(view.y0, 0);

        view.scroll(-1, &cfg);
        assert_eq!(view.y0, 0);
    }
}

#[test]
fn scroll_five_lines() {
    for (mode, delta, expected) in [
        (LastPageLines::Unspecified, 3, 3),
        (LastPageLines::Unspecified, 9, 0),
        (LastPageLines::OneBlank, 3, 3),
        (LastPageLines::OneBlank, 9, 3),
        (LastPageLines::OneNotBlank, 3, 3), // testing normal scroll
        (LastPageLines::OneNotBlank, 9, 4), // and scroll down limit
    ] {
        let cfg = TextConfig {
            last_page: mode,
            ..Default::default()
        };

        let mut view = TextViewer::new(String::from("1\n2\n3\n4\n5"), 3, 3);
        view.scroll(delta, &cfg);
        assert_eq!(view.y0, expected);
    }
}

impl TextChange {
    pub fn apply(&self, text: &mut String) {
        // `at` and `before` are byte offsets/lengths, so this slices on char boundaries.
        text.replace_range(self.at..self.at + self.before.len(), &self.after);
    }
}

impl TextSelection {
    fn selected_range(&self) -> (usize, usize) {
        if self.len >= 0 {
            (self.pos, self.pos + self.len as usize)
        } else {
            (self.pos - (-self.len as usize), self.pos)
        }
    }
    pub fn on_char(&mut self, text: &String, c: char) -> TextChange {
        let (at, before) = if self.len == 0 {
            (self.pos, String::new())
        } else {
            let (start, finish) = self.selected_range();
            self.len = 0;
            (start, text[start..finish].to_string())
        };
        self.pos = at + c.len_utf8();
        TextChange {
            at,
            before,
            after: c.into(),
        }
    }

    pub fn on_delete(&mut self, text: &String, backspace: bool) -> Option<TextChange> {
        if self.len != 0 {
            let (start, finish) = self.selected_range();
            self.pos = start;
            self.len = 0;
            Some(TextChange {
                at: start,
                before: text[start..finish].to_string(),
                after: String::new(),
            })
        } else if !backspace {
            if self.pos >= text.len() {
                return None;
            }
            // delete the whole char at the cursor, not a single byte
            let ch = text[self.pos..].chars().next().unwrap();
            let end = self.pos + ch.len_utf8();
            Some(TextChange {
                at: self.pos,
                before: text[self.pos..end].to_string(),
                after: String::new(),
            })
        } else {
            if self.pos == 0 {
                return None;
            }
            // step back over the whole char preceding the cursor
            let ch = text[..self.pos].chars().next_back().unwrap();
            self.pos -= ch.len_utf8();
            let end = self.pos + ch.len_utf8();
            Some(TextChange {
                at: self.pos,
                before: text[self.pos..end].to_string(),
                after: String::new(),
            })
        }
    }

    pub fn move_x(&mut self, cfg: &TextConfig, text_lines: &TextLines, mut delta: isize, select: bool) {
        if self.len != 0 && !select {
            let (start, finish) = self.selected_range();
            self.pos = if delta >= 0 { finish } else { start };
            self.len = 0;
            return;
        }

        // Step horizontally over whole characters, working directly in byte offsets and
        // clamping to the current visual line (matching the previous fixed-row behavior).
        let text = &text_lines.text;
        let (line_start, line_end) = text_lines.line_bounds(cfg, self.pos);
        let mut new_pos = self.pos;
        if delta >= 0 {
            while delta > 0 && new_pos < line_end {
                let ch = text[new_pos..].chars().next().unwrap();
                new_pos += ch.len_utf8();
                delta -= 1;
            }
        } else {
            while delta < 0 && new_pos > line_start {
                let ch = text[..new_pos].chars().next_back().unwrap();
                new_pos -= ch.len_utf8();
                delta += 1;
            }
        }

        if select {
            let delta_pos = new_pos as isize - self.pos as isize;
            self.len -= delta_pos;
        } else {
            self.len = 0;
        }

        self.pos = new_pos;
        self.x_pref = text_lines.to2d(cfg, self.pos).0;
    }
    pub fn move_y(&mut self, cfg: &TextConfig, text_lines: &TextLines, delta: isize, select: bool) {
        let (_, mut y) = text_lines.to2d(cfg, self.pos);
        if delta >= 0 {
            y += delta as usize;
        } else {
            if y >= -delta as usize {
                y -= -delta as usize;
            }
        }

        let new_pos = text_lines.to1d(cfg, (self.x_pref, y));

        if select {
            let delta_pos = new_pos as isize - self.pos as isize;
            self.len -= delta_pos;
        } else {
            self.len = 0;
        }

        self.pos = new_pos;
    }
}

impl Ord for TextSelection {
    fn cmp(&self, other: &TextSelection) -> Ordering {
        // last selection in the text will be processed first
        self.pos.cmp(&other.pos).reverse()
    }
}

impl PartialOrd for TextSelection {
    fn partial_cmp(&self, other: &TextSelection) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl TextEditor {
    pub fn new(text: String, window_width: usize, window_height: usize) -> Self {
        TextEditor {
            view: TextViewer::new(text, window_width, window_height),
            active_cursor_index: 0,
            selected: vec![TextSelection::default()],
            history: (vec![], vec![]),
        }
    }
    pub fn add_selection(&mut self, cfg: &TextConfig, pos: usize, len: isize) {
        self.selected.push(TextSelection {
            pos,
            len,
            x_pref: self.view.lines.to2d(cfg, pos).0,
        });
        self.selected.sort_unstable();

        // TODO join overlaped
        //self.selected.dedup();
    }

    // TODO    pub fn on_move_next_word(&mut self) { todo!(); }
    // TODO    pub fn on_move_prev_word(&mut self) { todo!(); }

    pub fn on_move_x(&mut self, cfg: &TextConfig, delta: isize, select: bool) {
        for sel in &mut self.selected {
            sel.move_x(cfg, &self.view.lines, delta, select);
        }
    }
    pub fn on_move_y(&mut self, cfg: &TextConfig, delta: isize, select: bool) {
        for sel in &mut self.selected {
            sel.move_y(cfg, &self.view.lines, delta, select);
        }
    }

    pub fn on_char(&mut self, cfg: &TextConfig, c: char) {
        let mut changes = TextChangeSet::new(TextChangeType::Type);

        let mut inserted = Vec::with_capacity(self.selected.len());
        for sel in &mut self.selected {
            let change = sel.on_char(&self.view.lines.text, c);
            sel.x_pref = self.view.lines.to2d(cfg, sel.pos).0;

            change.apply(&mut self.view.lines.text);
            inserted.push(change.after.len());
            changes.changes.push(change);
        }

        // shift selection if a new chars inserted before it
        let mut s = 0usize;
        inserted = inserted
            .into_iter()
            .rev()
            .map(|x| {
                let old = s;
                s += x;
                old
            })
            .collect();
        inserted.reverse();
        for i in 0..self.selected.len() {
            //let offset: usize = inserted[i + 1..].into_iter().sum();
            //self.selected[i].pos += offset;
            self.selected[i].pos += inserted[i];
        }

        self.history.0.push(changes);
        self.view.lines.starts.replace(vec![]);
    }

    pub fn on_delete(&mut self, cfg: &TextConfig, backspace: bool) {
        let mut changes = TextChangeSet::new(TextChangeType::Delete);

        for sel in &mut self.selected {
            if let Some(change) = sel.on_delete(&self.view.lines.text, backspace) {
                changes.changes.push(change);
                sel.x_pref = self.view.lines.to2d(cfg, sel.pos).0;
            }
        }

        if !changes.changes.is_empty() {
            //            for change in changes.changes.iter().rev() {
            for change in &changes.changes {
                change.apply(&mut self.view.lines.text);
            }
            self.history.0.push(changes);
            self.view.lines.starts.replace(vec![]);
        }
    }
}

#[test]
fn type_text() {
    let mut edit = TextEditor::new(String::new(), 80, 24);
    let cfg = TextConfig::default();
    edit.on_char(&cfg, 'x');
    assert_eq!(edit.view.lines.text, "x");
    edit.on_char(&cfg, 'y');
    assert_eq!(edit.view.lines.text, "xy");
}

#[test]
fn type_unicode_text() {
    let mut edit = TextEditor::new("й1".into(), 80, 24);
    let cfg = TextConfig::default();
    edit.on_move_x(&cfg, 1, false);
    edit.on_char(&cfg, '2');
    assert_eq!(edit.view.lines.text, "й21");
}


#[test]
fn type_text_vertically() {
    let mut edit = TextEditor::new(String::from("111\n222"), 80, 24);
    let cfg = TextConfig::default();
    edit.on_char(&cfg, 'x');
    assert_eq!(edit.view.lines.text, "x111\n222");
    edit.on_move_y(&cfg, 1, false);
    edit.on_char(&cfg, 'y');
    assert_eq!(edit.view.lines.text, "x111\n2y22");
}

#[test]
fn type_text_three_cursor() {
    let mut edit = TextEditor::new(String::from("1234567890"), 80, 24);
    let cfg = TextConfig::default();
    edit.add_selection(&cfg, 3, 0);
    edit.add_selection(&cfg, 6, 0);
    edit.on_char(&cfg, 'x');
    assert_eq!(edit.view.lines.text, "x123x456x7890");
    edit.on_char(&cfg, 'y');
    assert_eq!(edit.view.lines.text, "xy123xy456xy7890");
}

#[test]
fn type_over_selection() {
    let mut edit = TextEditor::new(String::from("1234567890"), 80, 24);
    let cfg = TextConfig::default();
    *edit.selected.last_mut().unwrap() = TextSelection {
        pos: 1,
        len: 5,
        x_pref: 0,
    };
    edit.on_char(&cfg, 'x');
    assert_eq!(edit.view.lines.text, "1x7890");
    edit.on_char(&cfg, 'y');
    assert_eq!(edit.view.lines.text, "1xy7890");
}

#[test]
fn delete_char() {
    let mut edit = TextEditor::new(String::from("1234567890"), 80, 24);
    let cfg = TextConfig::default();
    edit.on_delete(&cfg, false);
    assert_eq!(edit.view.lines.text, "234567890");
    edit.on_delete(&cfg, false);
    assert_eq!(edit.view.lines.text, "34567890");
    edit.on_delete(&cfg, true);
    assert_eq!(edit.view.lines.text, "34567890");

    edit.on_move_x(&cfg, 6, false);
    edit.on_delete(&cfg, true);
    assert_eq!(edit.view.lines.text, "3456790");
    edit.on_delete(&cfg, false);
    assert_eq!(edit.view.lines.text, "345670");
    edit.on_delete(&cfg, false);
    assert_eq!(edit.view.lines.text, "34567");
    edit.on_delete(&cfg, false);
    assert_eq!(edit.view.lines.text, "34567");
}

#[test]
fn delete_unicode_text() {
    let mut edit = TextEditor::new("абв".into(), 80, 24);
    let cfg = TextConfig::default();

    edit.on_move_x(&cfg, 3, false);

    edit.on_delete(&cfg, true);
    assert_eq!(edit.view.lines.text, "аб");
    edit.on_delete(&cfg, true);
    assert_eq!(edit.view.lines.text, "а");
    edit.on_delete(&cfg, true);
    assert_eq!(edit.view.lines.text, "");
}


#[test]
fn delete_selection() {
    let mut edit = TextEditor::new(String::from("1234567890"), 80, 24);
    let cfg = TextConfig::default();
    edit.on_move_x(&cfg, 6, false);
    edit.on_move_x(&cfg, -3, true);
    edit.on_delete(&cfg, false);
    assert_eq!(edit.view.lines.text, "1237890");
}

#[test]
fn delete_multi_cursor() {
    let mut edit = TextEditor::new(String::from("1234567890"), 80, 24);
    let cfg = TextConfig::default();
    edit.add_selection(&cfg, 3, 0);
    //    edit.selected = vec![
    //        TextSelection {
    //            pos: 0,
    //            len: 0,
    //            x_pref: 0,
    //        },
    //        TextSelection {
    //            pos: 3,
    //            len: 0,
    //            x_pref: 3,
    //        },
    //    ];

    println!("{:?}", edit);

    edit.on_delete(&cfg, false);
    assert_eq!(edit.view.lines.text, "23567890");
}

#[test]
fn delete_multi_selection() {
    let mut edit = TextEditor::new(String::from("1234567890"), 80, 24);
    let cfg = TextConfig::default();
    edit.selected = vec![
        TextSelection {
            pos: 7,
            len: 2,
            x_pref: 0,
        },
        TextSelection {
            pos: 1,
            len: 2,
            x_pref: 0,
        },
    ];
    edit.on_delete(&cfg, false);
    assert_eq!(edit.view.lines.text, "145670");
}

#[test]
fn move_selection_x() {
    let mut edit = TextEditor::new(String::from("1234567890"), 80, 24);
    let cfg = TextConfig::default();
    edit.on_move_x(&cfg, 2, false);
    edit.on_char(&cfg, 'x');
    assert_eq!(edit.view.lines.text, "12x34567890");
    edit.on_move_x(&cfg, -1, false);
    edit.on_char(&cfg, 'y');
    assert_eq!(edit.view.lines.text, "12yx34567890");
}

#[test]
fn expand_selection_x() {
    for (delta, delta_with_selection, expected) in [
        (0, 1, "x234567890"),
        (1, 1, "1x34567890"),
        (5, 1, "12345x7890"),
        (5, -1, "1234x67890"),
        (5, -3, "12x67890"),
    ] {
        let mut edit = TextEditor::new(String::from("1234567890"), 80, 24);
        let cfg = TextConfig::default();
        edit.on_move_x(&cfg, delta, false);
        edit.on_move_x(&cfg, delta_with_selection, true);
        edit.on_char(&cfg, 'x');
        assert_eq!(edit.view.lines.text, expected);
    }
}

#[test]
fn shrink_selection_x() {
    let mut edit = TextEditor::new(String::from("1234567890"), 80, 24);
    let cfg = TextConfig::default();
    edit.on_move_x(&cfg, 1, false);
    edit.on_move_x(&cfg, 3, true);
    edit.on_move_x(&cfg, -1, true);
    edit.on_char(&cfg, 'x');
    assert_eq!(edit.view.lines.text, "1x4567890");
}

#[test]
fn unselect_x() {
    {
        let mut edit = TextEditor::new(String::from("1234567890"), 80, 24);
        let cfg = TextConfig::default();
        edit.on_move_x(&cfg, 2, false);
        edit.on_move_x(&cfg, 2, true);
        edit.on_move_x(&cfg, 1, false);
        edit.on_char(&cfg, 'x');
        assert_eq!(edit.view.lines.text, "1234x567890");
    }
    {
        let mut edit = TextEditor::new(String::from("1234567890"), 80, 24);
        let cfg = TextConfig::default();
        edit.on_move_x(&cfg, 2, false);
        edit.on_move_x(&cfg, 2, true);
        edit.on_move_x(&cfg, -1, false);
        edit.on_char(&cfg, 'x');
        assert_eq!(edit.view.lines.text, "12x34567890");
    }
}

#[test]
fn move_selection_y() {
    let mut edit = TextEditor::new(String::from("123\n\t456\n789"), 80, 24);
    let cfg = TextConfig::default();
    edit.on_move_x(&cfg, 1, false);
    edit.on_move_y(&cfg, 1, false);
    edit.on_move_y(&cfg, 1, false);
    edit.on_char(&cfg, 'x');
    assert_eq!(edit.view.lines.text, "123\n\t456\n7x89");
}

#[test]
fn expand_selection_y() {
    let mut edit = TextEditor::new(String::from("123\n456\n789"), 80, 24);
    let cfg = TextConfig::default();
    edit.on_move_y(&cfg, 1, true);
    edit.on_char(&cfg, 'x');
    assert_eq!(edit.view.lines.text, "x456\n789");
}

#[test]
fn unselect_y() {
    for (delta, expected) in [(1, "123\n456\n78x9"), (-1, "12x3\n456\n789")] {
        let mut edit = TextEditor::new(String::from("123\n456\n789"), 80, 24);
        let cfg = TextConfig::default();
        edit.on_move_y(&cfg, 1, false);
        edit.on_move_x(&cfg, 2, true);
        edit.on_move_y(&cfg, delta, false);
        edit.on_char(&cfg, 'x');
        assert_eq!(edit.view.lines.text, expected);
    }
}

//fn main() {    println!("Hello, world!");}
