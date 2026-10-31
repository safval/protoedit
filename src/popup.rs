// The upper UI layer: a modal box drawn over the layouts, either a message with
// buttons (MessageBox) or a list to choose a value from (enum names, etc.).
// The popup knows nothing about the data: every item carries the CommandResult
// the App runs when the item is chosen, as if a layout returned it, so a chosen
// enum value is an ordinary ChangeData

use crate::view::{CommandResult, ScreenLine, TextStyle, UserCommand};

pub struct PopupItem {
    pub label: String,
    pub action: CommandResult, // run by the App when the item is chosen
}

impl PopupItem {
    pub fn new(label: &str, action: CommandResult) -> PopupItem {
        PopupItem { label: label.to_string(), action }
    }
}

pub struct Popup {
    title: String,
    text: Vec<String>,     // message lines, empty for a list
    items: Vec<PopupItem>, // list items or buttons
    buttons: bool,         // true: a row of buttons (←/→), false: a vertical list (↑/↓)
    selected: usize,
    // first visible item of a list higher than the screen; kept by draw so the
    // selected item stays visible
    scroll: usize,
    // how many list items fit the box, known after draw; the step of PageUp/PageDown
    page: usize,
    // the list opens below (or above) this position of the selected layout,
    // in the coordinates of ViewLayout::get_text_edit_cursor; None: centered
    anchor: Option<(u16, usize)>,
}

impl Popup {
    pub fn message(title: &str, text: &str) -> Popup {
        Self::confirm(title, text, vec![PopupItem::new("OK", CommandResult::None)])
    }
    pub fn confirm(title: &str, text: &str, buttons: Vec<PopupItem>) -> Popup {
        Popup {
            title: title.to_string(),
            text: text.lines().map(String::from).collect(),
            items: buttons,
            buttons: true,
            selected: 0,
            scroll: 0,
            page: 1,
            anchor: None,
        }
    }
    pub fn list(title: &str, items: Vec<PopupItem>, selected: usize, anchor: Option<(u16, usize)>) -> Popup {
        Popup {
            title: title.to_string(),
            text: vec![],
            selected: selected.min(items.len().saturating_sub(1)),
            items,
            buttons: false,
            scroll: 0,
            page: 1,
            anchor,
        }
    }
    pub fn anchor(&self) -> Option<(u16, usize)> { self.anchor }

    // Some(result) closes the popup and the App runs the result,
    // None keeps the popup open
    pub fn on_command(&mut self, command: UserCommand) -> Option<CommandResult> {
        let last = self.items.len().saturating_sub(1);
        match command {
            UserCommand::CollapsedToggle => { // Enter: choose the selected item
                if self.items.is_empty() { return Some(CommandResult::None) }
                return Some(self.items.swap_remove(self.selected).action);
            }
            UserCommand::Exit => return Some(CommandResult::None), // Esc: cancel
            UserCommand::ScrollVertically(delta) if !self.buttons => {
                // PageUp/PageDown come with the screen height, the list moves by its own page
                let step = self.page.saturating_sub(1).max(1) as isize;
                self.selected = self.selected.saturating_add_signed(delta.clamp(-step, step)).min(last);
            }
            UserCommand::ScrollHorizontally(delta) if self.buttons => {
                self.selected = self.selected.saturating_add_signed(delta as isize).min(last);
            }
            UserCommand::Home => self.selected = 0,
            UserCommand::End => self.selected = last,
            UserCommand::KeyPress(c) => { // jump to the next item starting with the char
                let c = c.to_lowercase().next().unwrap_or(c);
                let n = self.items.len();
                for i in 1..=n {
                    let index = (self.selected + i) % n;
                    if self.items[index].label.to_lowercase().starts_with(c) {
                        self.selected = index;
                        break;
                    }
                }
            }
            _ => {}
        }
        None
    }

    // draw the box over the screen lines (the screen below the top line);
    // anchor is the screen position of self.anchor resolved by the App
    pub fn draw(&mut self, lines: &mut [ScreenLine], width: u16, anchor: Option<(u16, usize)>) {
        let width = width as usize;
        let height = lines.len();
        if width < 6 || height < 3 { return }

        let buttons_len: usize = self.items.iter().map(|item| item.label.chars().count() + 2).sum::<usize>()
            + 2 * self.items.len().saturating_sub(1);
        let mut content_width = self.title.chars().count() + 2;
        content_width = content_width.max(self.text.iter().map(|s| s.chars().count()).max().unwrap_or(0));
        content_width = content_width.max(if self.buttons { buttons_len } else {
            self.items.iter().map(|item| item.label.chars().count()).max().unwrap_or(0)
        });
        let box_width = (content_width + 4).min(width); // borders and a space padding at both sides
        let inner_width = box_width - 4;

        let rows = if self.buttons {
            if self.text.is_empty() { 1 } else { self.text.len() + 2 } // text, empty line, buttons
        } else { self.items.len().max(1) };
        let wanted_height = rows + 2;

        let centered = |box_height: usize| ((width - box_width) / 2, (height - box_height) / 2, box_height);
        let (x, y, box_height) = match anchor {
            Some((ax, ay)) if !self.buttons && ay < height => {
                // a list opens below the anchored value, or above it if there is more room
                let below = height - ay - 1;
                let above = ay;
                let x = (ax as usize).saturating_sub(2); // align the item text with the value
                if wanted_height <= below || (below >= above && below >= 3) {
                    (x, ay + 1, wanted_height.min(below))
                } else if above >= 3 {
                    let box_height = wanted_height.min(above);
                    (x, ay - box_height, box_height)
                } else {
                    centered(wanted_height.min(height))
                }
            }
            _ => centered(wanted_height.min(height)),
        };
        let x = x.min(width - box_width);
        let visible = box_height - 2;

        if !self.buttons {
            self.page = visible;
            if self.selected < self.scroll { self.scroll = self.selected }
            if self.selected >= self.scroll + visible { self.scroll = self.selected + 1 - visible }
            self.scroll = self.scroll.min(self.items.len().saturating_sub(visible));
        }

        for line in lines[y..y + box_height].iter_mut() {
            line.fix_length(width as u16);
        }

        // top border with the title
        let title = truncate(&self.title, inner_width.saturating_sub(2));
        let top = if title.is_empty() { String::new() } else { format!(" {} ", title) };
        let top = format!("┌─{}{}┐", top, "─".repeat(box_width - 3 - top.chars().count()));
        put(&mut lines[y], x, &top, TextStyle::Popup);

        for row in 0..visible {
            let line = &mut lines[y + 1 + row];
            put(line, x, &format!("│{}│", " ".repeat(box_width - 2)), TextStyle::Popup);
            if self.buttons {
                if row < self.text.len() {
                    put(line, x + 2, &truncate(&self.text[row], inner_width), TextStyle::Popup);
                } else if row == rows - 1 {
                    let mut bx = x + 2 + inner_width.saturating_sub(buttons_len) / 2;
                    for (index, item) in self.items.iter().enumerate() {
                        let label = format!("[{}]", item.label);
                        if bx + label.chars().count() > x + box_width - 2 { break } // no room on a narrow screen
                        let style = if index == self.selected { TextStyle::PopupSelected } else { TextStyle::Popup };
                        bx = put(line, bx, &label, style) + 2;
                    }
                }
            } else if let Some(item) = self.items.get(self.scroll + row) {
                let label = format!(" {:<inner_width$} ", truncate(&item.label, inner_width));
                let style = if self.scroll + row == self.selected { TextStyle::PopupSelected } else { TextStyle::Popup };
                put(line, x + 1, &label, style);
            }
        }
        // the list continues above or below the box
        if !self.buttons && self.scroll > 0 {
            put(&mut lines[y + 1], x + box_width - 1, "▲", TextStyle::Popup);
        }
        if !self.buttons && self.scroll + visible < self.items.len() {
            put(&mut lines[y + visible], x + box_width - 1, "▼", TextStyle::Popup);
        }

        put(&mut lines[y + box_height - 1], x, &format!("└{}┘", "─".repeat(box_width - 2)), TextStyle::Popup);
    }
}

fn truncate(text: &str, len: usize) -> String {
    text.chars().take(len).collect()
}

// write the text over the line from the column, cut at the line end;
// returns the column after the text
fn put(line: &mut ScreenLine, mut x: usize, text: &str, style: TextStyle) -> usize {
    for c in text.chars() {
        if x >= line.0.len() { break }
        line.0[x] = (c, style);
        x += 1;
    }
    x
}
