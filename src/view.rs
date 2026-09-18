use std::string::String;
use std::cmp::{Ordering, PartialEq};
use std::collections::{HashMap, HashSet};
use std::fmt::{Debug, Formatter};
use std::{io, iter, mem};
use std::io::Write;
use crossterm::cursor::position;
use crossterm::event::{KeyEvent};
use crossterm::style;
use crossterm::style::Color;
use crate::proto::{FieldProtoPtr, ProtoData};
use crate::Selection;
use crate::trz::{Change, ChangeType};
use crate::wire::{FieldPath, FieldPos, FieldValue, MessageData, ScalarValue};
use crate::wire::ScalarValue::{BYTES, STR};
use crate::text_edit::*;

pub(crate) const MARGIN_RIGHT: u16 = 1;
pub(crate) const MARGIN_LEFT: u16 = 1;

#[derive(Debug, Clone)]
pub enum UserCommand
{
    Refresh,
    // move left - negative value, right - positive
    ScrollHorizontally(i8),
    // keys (page)up/down, or mouse wheel
    // move up - negative value, move down - positive
    ScrollVertically(isize),
    ScrollSibling(i8),
    // hotkeys: Shift+←/→, Shift+↑/↓ and Shift+Home/End
    // move the cursor of an in-place editor expanding the selection
    SelectHorizontally(i8),
    SelectVertically(isize),
    SelectHome,
    SelectEnd,
    // hotkeys: Alt+Shift+↑/↓
    // multi-cursor: add a cursor one line above/below the active one
    AddCursor(i8),
    // hotkeys: Ctrl+Z and Ctrl+Y (or Ctrl+Shift+Z)
    // for now only inside an in-place editor; app-level undo (trz.rs) is a future task
    Undo,
    Redo,
    ScrollToBottom,
    Home,
    End,
    // hotkey: 'C' comments visibility (enum CommentVisibility)
    CommentsVisibility,
    // hotkey: 'B'
    // show/hide binary data (read only)
    // the data shown only in tree mode, before main data line,
    // binary content (tag and length) explained, for example:
    // 002F8 : 0A 2:LEN 8C 35 2248 00 00 00 00 00 0F
    BinaryVisibility,
    // hotkey: 'P'
    // show/hide tyPe (by default shown only in a few cases)
    DataTypeVisibility,
    // hotkey: Enter/F5 on collapsed field name
    Exit,
    CollapsedToggle,
    // hotkey: 'T'
    // tree / table mode switch) (vert/horiz auto select by content)
    TableTreeToggle,
    // hotkey: 'V'
    // switch vertical or regular table, in table view
    TableVariant,
    // hotkeys: '>','<'
    // increase or decrease column count
    ChangeColumnCount(i8),
    // hotkey: 'F4'
    // field Order in table or message (enum FieldOrder)
    ChangeFieldOrder(FieldOrder),
    // hotkey: 'Ctrl+←', 'Ctrl+→'
    // shift repeated scalar or table column
    MoveField,
    // hotkeys: Del/Backspace/Ins
    DeleteData(bool), // false=delete, true=backspace
    InsertData,
    // hotkeys: 'E' ,'I'
    // supported file format depend on data types, show in UI
    // and detected by entered file name (txt, bin, pb, csv, tsv, json)
    //ExportData,
    //ImportData,
    // hotkey 'S', when selected column name of a repeated message in table mode
    // sort table by this column (a...z|z...a|as read from file)
    SortDataView,
    // not a command, just key pressed
    KeyPress(char),
    // internal commands, sent by Layouts (not bound to keys) when the cursor
    // moves from one layout to another: the old layout drops its editor cursors
    // and selection, the new one continues at the remembered column (the
    // preferred_column argument of on_command), so neighboring editors feel
    // like one text and neighboring value lines like one grid.
    // The bool of FocusGained is true when the cursor enters from the layout
    // below (moving up), so the cursor goes to the last line of the layout
    FocusLost,
    FocusGained(bool),
}

pub enum CommandResult {
    None,
    Redraw,
    ChangeData(Change),
    ShowMenu(Vec<String>),
    ShowMessage(String),
    ShowError(String),
    StartEdit(FieldPath, u16, u16),
    Exit, // close the application
}

// Data on the screen arranged in layouts, each layout displayed as one or more line
pub struct Layouts { // rename Document
    pub width: u16,
    pub height: u16,
    pub scroll: usize,
    pub items: Vec<LayoutParams>,
    pub file_path: std::path::PathBuf,
    pub proto: ProtoData,
    pub indents: Vec<u16>,
    pub top_layouts_count: usize,
    // the document is a stream of length-delimited records (--delimited),
    // saving must write the varint length prefixes back
    pub delimited: bool,
}

pub struct LayoutParams {
    // how many lines requires this layout on the screen.
    // NOTE: while an in-place editor is open this value is stale — calc_sizes runs
    // only after a CommandResult::ChangeData is applied (App::after_command sets
    // need_update_layout_height), and an open editor changes only its own state.
    // So after Enter adds a line in a string editor, get_screen already draws the
    // extra line, but Layouts::run_command(ScrollVertically) still uses the old
    // height for layout boundaries and can move the cursor out of the field
    // before the edit is committed with Esc.
    pub height: usize,
    pub path: FieldPath,
    // how many repeated data with the same id shown by this layout, starting from path
    pub amount: usize,
    pub layout: Option<Box<dyn ViewLayout>>,
    pub children_count: usize,
}

#[derive(Debug, PartialEq)]
pub enum LayoutType {
    Scalar,
    Bytes,
    Str,
    Message,
    Table,
    Collapsed,
}

// does not store data, only params how to display it
// in next versions: multiple cursor, selection and highlight (found item, etc.)
pub trait ViewLayout {
    fn layout_type(&self) -> LayoutType;
    // return how many screen lines take this layout
    fn calc_sizes(&mut self, root: &MessageData, path: &FieldPath, amount: usize, config: &LayoutConfig, width: u16, negotiator: &mut IndentsCalc) -> usize;
    // TODO first_line: usize, line_count: u16
    fn get_screen(&self, root: &MessageData, path: &FieldPath, amount: usize, width: u16, indent: u16, config: &LayoutConfig, cursor: Option<(u16, usize)>) -> ScreenLines;
    // the blinking cursor of the terminal will be shown in this position, position in this layout, not screen
    fn get_text_edit_cursor(&self) -> Option<(u16, usize)> { None }
    // preferred cursor column of an open in-place editor or of the selected value;
    // used to keep the column when the focus moves vertically between neighboring
    // fields (the selection itself is stored globally, not in the layout)
    fn get_preferred_column(&self, _cursor_x: u16, _cursor_pos: usize) -> Option<u16> { None }
    fn on_command(&mut self, root: &MessageData, path: &FieldPath, amount: usize, command: UserCommand, config: &LayoutConfig, width: u16, indent: u16, preferred_column: Option<u16>, cursor_x: &mut u16, cursor_pos: &mut usize) -> CommandResult;
    // get ids of children fields already shown in this layout
    fn get_consumed_fields(&self, root: &MessageData, path: &FieldPath, config: &LayoutConfig) -> HashSet<i32> { HashSet::new() }
    fn get_status_string(&self, cursor_x: u16, cursor_y: usize) -> String { String::new() }
}

fn on_command_default_handler(root: &MessageData, path: &FieldPath, amount: usize, command: UserCommand, config: &LayoutConfig, width: u16, indent: u16, preferred_column: Option<u16>, cursor_x: &mut u16, cursor_pos: &mut usize) -> CommandResult {
    match command {
        UserCommand::DeleteData(backspace) => {
            if *cursor_x == 0 && *cursor_pos == 0 {
                CommandResult::ChangeData(Change { path: path.clone(), action: ChangeType::Delete })
            } else { CommandResult::None }
        }

        UserCommand::ScrollVertically(mut delta) => {
            //            print!("cursor_pos (default_handler): {} -> ", cursor_pos);

            *cursor_pos = (*cursor_pos as isize + delta) as usize;

            //            println!("{}", cursor_pos);

            CommandResult::Redraw
        }
        UserCommand::Exit => CommandResult::Exit,
        _ => CommandResult::None
    }
}

// bool, enum, integral, or real value: single, none or repeated
// there are special layouts for text and hex field types
pub struct ScalarLayout {
    // screen placement of the values: one Vec per screen line, one
    // (start column, length) per value on that line
    spans: Vec<Vec<(u16, u16)>>,
    edit: Option<ScalarEditor>,
}

// in-place editor of one scalar (numeric or bool) value inside a ScalarLayout;
// it edits plain text, the field's FieldProto parses it on commit
#[derive(Debug)]
struct ScalarEditor {
    text: Vec<char>,
    cursor: usize, // position in text, 0..=text.len()
    index: usize,  // which of the layout's values is edited
    line: usize,   // layout line the value is shown on
    x_base: u16,   // offset of the value's first char in the value area of the line
    error: Option<String>, // failed commit message, shown in the status line
}
impl ScalarEditor {
    fn new(text: String, index: usize, line: usize, x_base: u16) -> ScalarEditor {
        let text: Vec<char> = text.chars().collect();
        ScalarEditor { cursor: text.len(), text, index, line, x_base, error: None }
    }
    fn text(&self) -> String { self.text.iter().collect() }
    fn on_char(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += 1;
        self.error = None;
    }
    fn on_delete(&mut self, backspace: bool) {
        if backspace {
            if self.cursor > 0 {
                self.cursor -= 1;
                self.text.remove(self.cursor);
            }
        } else if self.cursor < self.text.len() {
            self.text.remove(self.cursor);
        }
        self.error = None;
    }
    fn move_x(&mut self, delta: isize) {
        self.cursor = self.cursor.saturating_add_signed(delta).min(self.text.len());
    }
}
pub struct StringLayout {
    edit: Option<TextEditor>,
    //edit_cfg: Option<TextConfig>,
    visible_lines_count: usize, // TODO
}
pub struct BytesLayout {
    bytes_per_line: u16,
    data_size: usize,
    edit: Option<ByteEditor>,
    //visible_lines_count: usize, // TODO
}
#[derive(Debug)]
struct ByteEditor {
    pos: usize,
    first_nibble: Option<u8>,
}
impl ByteEditor {
    pub fn new(pos: usize) -> ByteEditor {
        ByteEditor { pos, first_nibble: None }
    }
    pub fn on_char(&mut self, c: char) -> Option<u8>
    {
        if let Some(prev_nibble) = self.first_nibble {
            if let Some(nibble) = ByteEditor::char_to_nibble(c) {
                self.pos += 1;
                self.first_nibble = None;
                return Some((prev_nibble << 4) + nibble);
            }
        } else {
            self.first_nibble = ByteEditor::char_to_nibble(c);
        }
        None
    }
    fn char_to_nibble(c: char) -> Option<u8> {
        match c {
            '0'..='9' => Some(c as u8 - '0' as u8),
            'a'..='f' => Some(c as u8 - 'a' as u8 + 10),
            'A'..='F' => Some(c as u8 - 'A' as u8 + 10),
            _ => None,
        }
    }
}

pub struct MessageLayout { // with columns or title only
    scroll: usize, // first visible column index
}
pub struct TableLayout { // for repeated messages
    vertical: bool,
    scroll: (usize, usize), // column and row indexes of top-left visible cell
}

pub struct CollapsedLayout {
    display_size: usize,
}

pub enum CommentVisibility {
    Hidden,
    Multiline, // before data, possible multiline
    Inline,    // in the same line, after data and type, only one line of comment
}

#[derive(PartialEq, Debug, Clone)]
pub enum FieldDataViewFormat {
    Formated, // show data formated according proto file
    Decimal, // show integers as decimal
    Hex, // show integers as hex
}
#[derive(PartialEq, Debug, Clone)]
pub enum FieldOrder {
    Proto,  // as in proto file (default)
    Wire,   // as the data read from the file, repeated may be in several groups
    ByName, // alphabetically by the name of the field
    ById,   // by numerical field id
}

pub struct LayoutConfig {
    pub show_comments: CommentVisibility,
    pub show_binary: bool,
    pub show_data_types: bool,
    pub field_order: FieldOrder,
    pub messages: HashMap<String, MessageLayoutConfig>,
    pub format: FieldDataViewFormat,
    pub text_edit_cfg: TextConfig,
}

// How to show a message or table of a certain type
pub struct MessageLayoutConfig {
    columns: Vec<i32>,
    columns_width: Vec<u16>,
}

#[derive(PartialEq, Debug, Copy, Clone)]
#[repr(u8)]
pub enum TextStyle {
    Comment,
    Binary,
    Filename,
    FieldName, // column header
    SelectedFieldName,
    FieldIndex, // left column
    SelectedFieldIndex,
    Value, // data content
    SelectedValue,
    EditCursor, // a non-active cursor of a multi-cursor editor (the active one blinks)
    DefaultValue,
    DataSize, // size of collapsed field
    Typename, // name of scalar type
    SelectedTypename, // for oneof
    Divider,
    Bookmark,
    TopLine, // top line with different status information
    Unknown,
}

pub struct ScreenLine(pub Vec<(char, TextStyle)>);

impl Default for LayoutConfig {
    fn default() -> Self {
        LayoutConfig {
            show_comments: CommentVisibility::Hidden,
            show_binary: false,
            show_data_types: false,
            field_order: FieldOrder::Proto,
            messages: HashMap::new(),
            format: FieldDataViewFormat::Formated,
            text_edit_cfg: TextConfig::default(),
        }
    }
}

impl CommentVisibility {
    pub fn next(&self) -> CommentVisibility {
        match self {
            CommentVisibility::Hidden => CommentVisibility::Inline,
            CommentVisibility::Inline => CommentVisibility::Multiline,
            CommentVisibility::Multiline => CommentVisibility::Hidden,
        }
    }
}
impl FieldOrder {
    const VARIANTS: [FieldOrder; 5] = [FieldOrder::Proto, FieldOrder::Wire, FieldOrder::ByName, FieldOrder::ById, FieldOrder::Proto];
    const VARIANTS_NAMES: [char; 4] = ['P', 'W', 'N', 'I'];
    pub fn next(&self) -> FieldOrder {
        let pos = FieldOrder::VARIANTS.iter().position(|item| { *item == *self }).unwrap();
        FieldOrder::VARIANTS[pos + 1].clone()
    }
    pub fn prev(&self) -> FieldOrder {
        let pos = FieldOrder::VARIANTS.iter().rposition(|item| { *item == *self }).unwrap();
        FieldOrder::VARIANTS[pos - 1].clone()
    }
    pub fn first_letter(&self) -> char {
        let pos = FieldOrder::VARIANTS.iter().position(|item| { *item == *self }).unwrap();
        Self::VARIANTS_NAMES[pos]
    }
}

// One line shown on the terminal screen
impl ScreenLine {
    pub fn new(width: u16) -> ScreenLine { ScreenLine(Vec::with_capacity(width as usize)) }

    pub fn add_string(&mut self, text: String, style: TextStyle) {
        let mut new_item = text.chars().map(|c| (c, style)).collect::<Vec<(char, TextStyle)>>();
        self.0.append(&mut new_item);
    }

    pub fn add_field_name(&mut self, text: String, indent: u16, cursor: &Option<(u16, usize)>) {
        self.add_first_column_item([TextStyle::FieldName, TextStyle::SelectedFieldName], text, indent, cursor, 0);
    }
    pub fn add_value_address(&mut self, text: String, indent: u16, cursor: &Option<(u16, usize)>, cursor_pos: usize) {
        self.add_first_column_item([TextStyle::FieldIndex, TextStyle::SelectedFieldIndex], text, indent, cursor, cursor_pos);
    }
    fn add_first_column_item(&mut self, styles: [TextStyle; 2], text: String, indent: u16, cursor: &Option<(u16, usize)>, cursor_pos: usize) {
        let mut selected = false;
        if let Some((x, pos)) = cursor {
            selected = *x == 0 && *pos == cursor_pos;
        }
        if selected {
            for _ in 1..indent as usize - text.len() {
                self.0.push((' ', styles[0]));
            }
            self.0.push((' ', styles[1]));
            self.add_string(text, styles[1]);
            self.0.push((':', styles[1]));
        } else {
            let width = indent as usize;
            self.add_string(format!("{text:>width$}"), styles[0]);
            self.0.push((':', TextStyle::Divider));
        }
    }
    pub fn add_field_size(&mut self, value: usize, screen_width: u16) {
        //self.data_size = Some(value);
        //let width = screen_width - self.0.len() as u16 - MARGIN_RIGHT;
        let s = format!(" ... {}", value);
        self.add_string(s, TextStyle::DataSize);
    }
    pub fn add_typename(&mut self, field_def: FieldProtoPtr, screen_width: u16, empty: bool) {
        let mut text = field_def.typename();
        if field_def.repeated() { text = text + "*" }
        if empty { text = "-".to_string() + text.as_str() }
        let max_allowed_len = (screen_width - MARGIN_RIGHT) as usize - text.len();
        if self.0.len() > max_allowed_len {
            self.0.truncate(max_allowed_len);
        }
        let width = (screen_width - MARGIN_RIGHT) as usize - self.0.len();
        self.add_string(format!("{text:>width$}"), TextStyle::Typename);
        for _ in 0..MARGIN_RIGHT { self.0.push((' ', TextStyle::Typename)); }
    }

    pub fn fix_length(&mut self, len: u16) {
        let len = len as usize;
        match self.0.len().cmp(&len) {
            Ordering::Less => {
                let mut spaces = iter::repeat_n((' ', TextStyle::Divider), len - self.0.len()).collect();
                self.0.append(&mut spaces);
            }
            Ordering::Greater => {
                self.0.truncate(len);
            }
            Ordering::Equal => {}
        }
    }
}
pub struct ScreenLines(pub Vec<ScreenLine>);
impl ScreenLines {
    pub fn new() -> ScreenLines { ScreenLines(Vec::with_capacity(100)) }
    pub fn append(&mut self, other: &mut ScreenLines) { self.0.append(&mut other.0); }
}

pub struct IndentsCalc {
    level_indents: Vec<u16>,
}

impl IndentsCalc {
    const NEXT_LEVEL_INDENT: u16 = 2;

    pub fn new() -> IndentsCalc {
        IndentsCalc {
            level_indents: Vec::with_capacity(8)
        }
    }

    pub fn new_for_update(indents: Vec<u16>) -> IndentsCalc {
        IndentsCalc {
            level_indents: indents
        }
    }

    pub fn add(&mut self, first_column_width: usize /* TODO u16*/, mut level: usize) -> u16 {
        debug_assert!(level >= 1);
        level -= 1;
        while self.level_indents.len() <= level {
            let new_indent = self.level_indents.last().unwrap_or(&0);
            self.level_indents.push(Self::NEXT_LEVEL_INDENT + *new_indent);
        }

        let new_width = MARGIN_LEFT + first_column_width as u16;
        if self.level_indents[level] < new_width {
            self.level_indents[level] = new_width;
            for i in level + 1..self.level_indents.len() {
                self.level_indents[i] = self.level_indents[i - 1] + Self::NEXT_LEVEL_INDENT;
            }
        }
        self.level_indents[level]
    }
}

impl Into<Vec<u16>> for IndentsCalc {
    fn into(self) -> Vec<u16> {
        self.level_indents
    }
}


impl ScalarLayout {
    const MARGIN: u16 = MARGIN_LEFT + MARGIN_RIGHT;

    fn new() -> Self {
        ScalarLayout { spans: vec![], edit: None }
    }
    fn add_scalar_value(line: &mut ScreenLine, value: &ScalarValue, def: &FieldProtoPtr, config: &LayoutConfig, selected: bool) {
        line.0.push((' ', TextStyle::Divider));
        let style = if selected { TextStyle::SelectedValue } else { TextStyle::Value };
        line.add_string(Self::scalar_to_string(value, def, config), style);
    }
    fn scalar_to_string(value: &ScalarValue, def: &FieldProtoPtr, config: &LayoutConfig) -> String {
        if let ScalarValue::ENUM(value) = value {
            if let Some(text) = def.get_enum_name_by_index(*value) {
                text.to_string()
            } else {
                format!("?{}", *value)
            }
        } else {
            //            if config.hex {
            //                format!("{:X}", value) // TODO
            //            } else {
            format!("{}", value)
            //            }

        }
    }

    fn get_value_spans(&self, full_width: u16, indent: u16, def: &FieldProtoPtr, msg: &MessageData, path: &FieldPath, amount: usize, config: &LayoutConfig) -> Vec<Vec<(u16, u16)>> {
        let mut avail_width = (full_width - indent - Self::MARGIN) as usize;
        if def.repeated() { avail_width -= 1 }
        avail_width -= def.typename().len();

        debug_assert!(amount > 0);
        let mut cur_len = 0;

        let mut spans = vec![];
        let mut line = vec![];

        if let Some(last_pos) = path.0.last() {
            for index in last_pos.index..last_pos.index + amount {
                if let Some(field) = msg.get_field(&([(last_pos.id, index).into()])) {
                    if let FieldValue::SCALAR(value) = &field.value {
                        let str_value = Self::scalar_to_string(value, def, config);
                        let len = str_value.len();
                        cur_len += len + 1;
                        if cur_len >= avail_width {
                            cur_len = len + 1;
                            spans.push(mem::take(&mut line));
                            avail_width = (full_width - indent - Self::MARGIN) as usize;
                        }
                        line.push(((cur_len - len - 1) as u16, len as u16));
                    }
                }
            }
            if !line.is_empty() { spans.push(line) }
        }

        spans
    }

    fn data_index_at_cursor(&self, cursor_x: u16, mut cursor_y: usize) -> usize {
        if cursor_x == 0 { return usize::MAX; } // selected field name, no data
        let at_line_start: usize = self.spans.iter().take(cursor_y).map(|line| line.len()).sum();
        at_line_start + cursor_x as usize - 1
    }

    fn cursor_at_data_index(&self, index: usize) -> (u16, usize) {
        let mut sum = 0;
        for line_index in 0..self.spans.len() {
            let line_len = self.spans[line_index].len();
            if sum + line_len >= index {
                return ((index - sum + 1) as u16, line_index);
            }
            sum += line_len;
        }
        (0, self.spans.len())
    }

    // the value of a line at the given screen column: a value is matched by its
    // span plus the following space, a column beyond the line end selects the
    // last value, an empty line the field name (0)
    fn value_at_column(line: &[(u16, u16)], column: u16) -> u16 {
        for (i, &(start, len)) in line.iter().enumerate() {
            if column < start + len + 1 { return (i + 1) as u16 }
        }
        line.len() as u16
    }

    // open the in-place editor for the value under the cursor;
    // prefill puts the current value text into the editor (Enter),
    // without it the editor opens empty to replace the value with the typed text
    fn open_editor(&mut self, root: &MessageData, path: &FieldPath, amount: usize, config: &LayoutConfig, cursor_x: u16, cursor_pos: usize, prefill: bool) -> bool {
        let Some(def) = root.get_field_definition(path) else { return false };
        if !def.editable() { return false }
        let index = self.data_index_at_cursor(cursor_x, cursor_pos);
        if index == usize::MAX || (amount > 0 && index >= amount) { return false }
        let Some(&(x_base, _)) = self.spans.get(cursor_pos).and_then(|line| line.get(cursor_x as usize - 1)) else { return false };

        let mut text =
            if amount == 0 { // the field is absent from the data, its default value is shown
                let FieldValue::SCALAR(value) = def.default() else { return false };
                Self::scalar_to_string(&value, &def, config)
            } else {
                let p = path.with_last_index(path.0.last().unwrap().index + index);
                let Some(field) = root.get_field(&p.0) else { return false };
                let FieldValue::SCALAR(value) = &field.value else { return false };
                Self::scalar_to_string(value, &def, config)
            };

        if !prefill { text.clear() }
        self.edit = Some(ScalarEditor::new(text, index, cursor_pos, x_base));
        true
    }

    // parse the editor text and close the editor turning the text into a data
    // change; empty text reverts the edit; on a parse error the editor stays open
    // showing the error, unless the focus is leaving (force) and it cannot stay —
    // then the edit is dropped
    fn commit_editor(&mut self, root: &MessageData, path: &FieldPath, amount: usize, force: bool) -> CommandResult {
        let Some(mut edit) = self.edit.take() else { return CommandResult::None };
        let text = edit.text();
        if text.is_empty() { return CommandResult::Redraw } // nothing typed: revert
        let Some(def) = root.get_field_definition(path) else { return CommandResult::Redraw };
        match def.parse(&text) {
            Ok(value) => {
                let path = path.with_last_index(path.0.last().unwrap().index + edit.index);
                if amount == 0 { // editing the shown default value creates the field
                    return CommandResult::ChangeData(Change { path, action: ChangeType::Insert(FieldValue::SCALAR(value)) });
                }
                if let Some(field) = root.get_field(&path.0) {
                    if let FieldValue::SCALAR(current) = &field.value {
                        if *current == value { return CommandResult::Redraw } // unchanged
                    }
                }
                CommandResult::ChangeData(Change { path, action: ChangeType::Overwrite(FieldValue::SCALAR(value)) })
            }
            Err(message) => {
                if !force {
                    edit.error = Some(message);
                    self.edit = Some(edit);
                }
                CommandResult::Redraw
            }
        }
    }
}
impl ViewLayout for ScalarLayout {
    fn layout_type(&self) -> LayoutType { LayoutType::Scalar }
    fn get_status_string(&self, cursor_x: u16, cursor_y: usize) -> String {
        //format!("/{}", self.amount)
        if let Some(edit) = &self.edit {
            if let Some(error) = &edit.error { return error.clone() }
        }
        String::new()
    }
    fn calc_sizes(&mut self, root: &MessageData, path: &FieldPath, amount: usize, config: &LayoutConfig, width: u16, negotiator: &mut IndentsCalc) -> usize {
        if let Some(field_proto) = root.get_field_definition(path) {
            let field_name_length = field_proto.name().len();
            let level = path.0.len();
            let indent = negotiator.add(field_name_length, level);

            let mut line_count = 1;
            if amount > 0 {
                let mut p = path.0.clone();
                p.pop();
                if let Some(msg) = root.get_submessage(&p) {
                    self.spans = self.get_value_spans(width, indent, &field_proto, msg, path, amount, config);
                    line_count = self.spans.len();
                }
            } else if let FieldValue::SCALAR(value) = field_proto.default() {
                // no data, the default value is shown; it can be selected and
                // edited (editing an absent field inserts it)
                self.spans = vec![vec![(0, Self::scalar_to_string(&value, &field_proto, config).len() as u16)]];
            }
            return line_count.max(1); // if no data, default value will be shown
        }
        panic!("cannot layout")
    }

    // TODO    fn index_by_coordinates(&self, root: &MessageData, path: &FieldPath, x: u16, y: usize) -> u16 {
    // TODO        todo!()
    // TODO    }
    // TODO    fn coordinates_by_index(&self, root: &MessageData, path: &FieldPath, x: u16, y: usize) -> u16 {
    // TODO        todo!()
    // TODO    }

    fn get_screen(&self, root: &MessageData, path: &FieldPath, amount: usize, width: u16, indent: u16, config: &LayoutConfig, cursor: Option<(u16, usize)>) -> ScreenLines {
        let mut lines = ScreenLines::new();
        let mut line = ScreenLine::new(width);
        if let Some(field_def) = root.get_field_definition(path) {
            line.add_field_name(field_def.name().clone(), indent, &cursor);


            let selected_index = cursor.map_or(usize::MAX, |(x, y)| self.data_index_at_cursor(x, y));

            if amount == 0 {
                if let Some(edit) = &self.edit {
                    line.0.push((' ', TextStyle::Divider));
                    line.add_string(edit.text(), TextStyle::SelectedValue);
                } else if let FieldValue::SCALAR(value) = field_def.default() {
                    // no data was read, show default value
                    Self::add_scalar_value(&mut line, &value, &field_def, config, selected_index == 0);
                }
            } else {
                let mut avail_width = (width - indent - Self::MARGIN) as usize;
                if field_def.repeated() { avail_width -= 1 }
                avail_width -= field_def.typename().len();

                debug_assert!(amount > 0);
                let mut cur_len = 0;
                let mut line_count = 1;
                let mut p = path.0.clone();
                for index in 0..amount {
                    if let Some(field) = root.get_field(&p) {
                        if let FieldValue::SCALAR(value) = &field.value {
                            let str_value = Self::scalar_to_string(value, &field_def, config);
                            let len = str_value.len();
                            cur_len += len + 1;
                            if cur_len >= avail_width {
                                cur_len = len + 1;
                                line_count += 1;

                                if lines.0.is_empty() {
                                    avail_width = (width - indent - Self::MARGIN) as usize;
                                    line.add_typename(field.def.clone(), width, false);
                                }

                                lines.0.push(line);
                                line = ScreenLine::new(width);
                                line.add_value_address(format!("{}", index), indent, &cursor, lines.0.len());
                            }
                            // the edited value shows the editor text instead of the data;
                            // the line breaks still follow the data lengths (line_lens)
                            match &self.edit {
                                Some(edit) if edit.index == index => {
                                    line.0.push((' ', TextStyle::Divider));
                                    line.add_string(edit.text(), TextStyle::SelectedValue);
                                }
                                _ => Self::add_scalar_value(&mut line, value, &field.def, config, selected_index == index),
                            }
                        }
                    }
                    p.last_mut().unwrap().index += 1;
                }
            }

            if lines.0.is_empty() {
                line.add_typename(field_def.clone(), width, amount == 0);
            }
            line.fix_length(width);
        }

        lines.0.push(line);
        lines
    }
    fn get_text_edit_cursor(&self) -> Option<(u16, usize)> {
        self.edit.as_ref().map(|edit| (edit.x_base + edit.cursor as u16, edit.line))
    }
    fn get_preferred_column(&self, cursor_x: u16, cursor_pos: usize) -> Option<u16> {
        if let Some(edit) = &self.edit {
            return Some(edit.x_base + edit.cursor as u16);
        }
        if cursor_x == 0 { return None } // the field name is selected, no column to keep
        self.spans.get(cursor_pos).and_then(|line| line.get(cursor_x as usize - 1)).map(|&(start, _)| start)
    }
    fn on_command(&mut self, root: &MessageData, path: &FieldPath, amount: usize, command: UserCommand, config: &LayoutConfig, width: u16, indent: u16, preferred_column: Option<u16>, cursor_x: &mut u16, cursor_pos: &mut usize) -> CommandResult
    {
        match command {
            UserCommand::KeyPress(c) => {
                let Some(def) = root.get_field_definition(path) else { return CommandResult::None };
                if !def.is_edit_char(c) { return CommandResult::None }
                if self.edit.is_none() {
                    // typing over a value replaces it: the editor opens empty
                    if !self.open_editor(root, path, amount, config, *cursor_x, *cursor_pos, false) {
                        return CommandResult::None;
                    }
                }
                self.edit.as_mut().unwrap().on_char(c);
                CommandResult::Redraw
            }
            UserCommand::CollapsedToggle => { // Enter: open the editor on the current value or commit it
                if self.edit.is_some() {
                    self.commit_editor(root, path, amount, false)
                } else if self.open_editor(root, path, amount, config, *cursor_x, *cursor_pos, true) {
                    CommandResult::Redraw
                } else { CommandResult::None }
            }
            UserCommand::Exit => { // Esc commits and closes the editor, without one it closes the app
                if self.edit.is_some() {
                    self.commit_editor(root, path, amount, false)
                } else { CommandResult::Exit }
            }
            UserCommand::FocusLost => // the cursor moved to another field
                self.commit_editor(root, path, amount, true),
            UserCommand::FocusGained(from_below) => {
                // select (not edit) the value at the remembered column, so vertical
                // movement keeps the screen column
                let Some(column) = preferred_column else { return CommandResult::None };
                let y = if from_below { self.spans.len().max(1) - 1 } else { 0 };
                if let Some(line) = self.spans.get(y) {
                    *cursor_x = Self::value_at_column(line, column);
                    *cursor_pos = y;
                    CommandResult::Redraw
                } else { CommandResult::None }
            }
            UserCommand::ScrollVertically(delta) => {
                // the editor cannot follow the cursor to another line: commit it
                let result = self.commit_editor(root, path, amount, true);
                *cursor_pos = (*cursor_pos as isize + delta) as usize;
                if *cursor_x > 0 {
                    // moving between the lines of this layout also aims at the
                    // remembered column
                    if let (Some(column), Some(new_line)) = (preferred_column, self.spans.get(*cursor_pos)) {
                        *cursor_x = Self::value_at_column(new_line, column);
                    }
                }
                if let CommandResult::ChangeData(_) = result { result } else { CommandResult::Redraw }
            }
            UserCommand::DeleteData(backspace) => {
                if let Some(edit) = &mut self.edit {
                    edit.on_delete(backspace);
                    CommandResult::Redraw
                } else if *cursor_x == 0 && *cursor_pos == 0 {
                    on_command_default_handler(root, path, amount, command, config, width, indent, preferred_column, cursor_x, cursor_pos)
                } else {
                    let index = self.data_index_at_cursor(*cursor_x, *cursor_pos);
                    if amount > 0 && index > 0 && index == amount - 1 {
                        // fix the cursor position after deleting last data item
                        (*cursor_x, *cursor_pos) = self.cursor_at_data_index(index - 1);
                    }
                    let path = path.with_last_index(path.0.last().unwrap().index + index);
                    self.spans.clear();
                    CommandResult::ChangeData(Change { path, action: ChangeType::Delete })
                }
            }
            UserCommand::InsertData => {
                if self.edit.is_some() { return CommandResult::None }
                let index = self.data_index_at_cursor(*cursor_x, *cursor_pos);
                let path = path.with_last_index(path.0.last().unwrap().index + index + 1);
                (*cursor_x, *cursor_pos) = self.cursor_at_data_index(index + 1);
                self.spans.clear();
                let def = root.get_field_definition(&path).unwrap();
                CommandResult::ChangeData(Change { path: path.clone(), action: ChangeType::Insert(def.default()) })
            }
            UserCommand::ScrollHorizontally(delta) => {
                if let Some(edit) = &mut self.edit {
                    edit.move_x(delta as isize);
                    CommandResult::Redraw
                } else if let Some(line) = self.spans.get(*cursor_pos) {
                    if delta > 0 {
                        *cursor_x = (*cursor_x + delta as u16).min(line.len() as u16);
                    } else { // delta < 0
                        let delta = (-delta as u16).min(*cursor_x);
                        *cursor_x -= delta;
                    }
                    CommandResult::Redraw
                } else { CommandResult::None }
            }
            UserCommand::Home => {
                if let Some(edit) = &mut self.edit {
                    edit.move_x(isize::MIN);
                } else {
                    *cursor_x = if *cursor_x == 1 { 0 } else { 1 };
                }
                CommandResult::Redraw
            }
            UserCommand::End => {
                if let Some(edit) = &mut self.edit {
                    edit.move_x(isize::MAX);
                } else if let Some(line) = self.spans.get(*cursor_pos) {
                    *cursor_x = line.len() as u16;
                }
                CommandResult::Redraw
            }
            _ => on_command_default_handler(root, path, amount, command, config, width, indent, preferred_column, cursor_x, cursor_pos)
        }
    }
}

impl StringLayout {
    const MARGIN: u16 = 8 + MARGIN_LEFT + MARGIN_RIGHT;
    fn get_lines_formated<'t>(&self, full_width: u16, indent: u16, repeated: bool, empty_field: bool, text: &'t String) -> Vec<(&'t str, bool)> {
        let mut res = vec![];

        let mut avail_width = (full_width - indent - Self::MARGIN) as usize;
        if repeated { avail_width -= 1 }
        if empty_field { avail_width -= 1 }

        for line in text.lines() {
            let mut start_pos = 0;
            let mut end_pos = line.len();
            loop {
                if avail_width < end_pos - start_pos {
                    end_pos = start_pos + avail_width;
                }

                // byte index 76 is not a char boundary; it is inside 'а' (bytes 75..77) of `исполняющий обязанности премьер-министра` note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
                res.push((&line[start_pos..end_pos], start_pos == 0));
                avail_width = (full_width - indent - 3) as usize;

                if end_pos >= line.len() { break; }
                start_pos = end_pos;
                end_pos = line.len();
            }
        }
        res
    }
}
impl ViewLayout for StringLayout {
    fn layout_type(&self) -> LayoutType { LayoutType::Str }
    fn calc_sizes(&mut self, root: &MessageData, path: &FieldPath, amount: usize, config: &LayoutConfig, width: u16, negotiator: &mut IndentsCalc) -> usize {

        // calculate width of first column as maximum length of field name and address
        let mut def: Option<FieldProtoPtr> = None;
        let mut value: Option<&String> = None;
        if let Some(field) = root.get_field(&path.0) {
            if let FieldValue::SCALAR(ScalarValue::STR(data)) = &field.value {
                def = Some(field.def.clone());
                value = Some(&data);
            }
        }
        if def.is_none() { // no data was read, get field name from proto file
            if let Some(field_def) = root.get_field_definition(path) {
                def = Some(field_def.clone());
            }
        }

        let mut line_count = 1;
        if let Some(field_def) = def {
            let indent = negotiator.add(field_def.name().len(), path.0.len());

            if let Some(text) = value {
                line_count = self.get_lines_formated(width, indent, field_def.repeated(), amount == 0, text).len();

                let mut address_len = 0;
                address_len = format!("{}", line_count).len() as u16;

                if address_len > indent {
                    negotiator.add(address_len as usize, path.0.len());
                    line_count = self.get_lines_formated(width, indent, field_def.repeated(), amount == 0, text).len();
                    // if line count changed, address length may be increased
                }
            }
        }
        return line_count.max(1);
    }

    fn get_screen(&self, root: &MessageData, path: &FieldPath, amount: usize, width: u16, indent: u16, config: &LayoutConfig, cursor: Option<(u16, usize)>) -> ScreenLines {
        let mut lines = vec![];
        let mut line = ScreenLine::new(width);

        // only the focused editor is drawn in editing style (`cursor` is Some just
        // for the selected layout); an unfocused editor goes to inactive mode and
        // is drawn like a regular field: quotes around a short text and the type
        // name at the end of the line
        let focused_edit = if cursor.is_some() { self.edit.as_ref() } else { None };

        if let Some(edit) = focused_edit {
            let mut line_number = 1;
            let mut line_number_changed = false;
            for indexes in edit.view.lines.starts_and_ends(&config.text_edit_cfg) {
                line = ScreenLine::new(width);

                if lines.is_empty() {
                    // first line starts with field name
                    if let Some(field_def) = root.get_field_definition(path) {
                        line.add_field_name(field_def.name().clone(), indent, &cursor);
                    }
                } else { // next lines starts with line number or empty if the number is not changed
                    let addr = if line_number_changed { format!("{}", line_number) } else { String::new() };
                    line.add_value_address(addr, indent, &cursor, lines.len());
                }

                line.0.push((' ', TextStyle::Divider));
                for (offset, c) in edit.view.lines.text[indexes.0..indexes.1].char_indices() {
                    let pos = indexes.0 + offset;
                    let style = if edit.is_other_cursor(pos) { TextStyle::EditCursor }
                        else if edit.is_selected(pos) { TextStyle::SelectedValue }
                        else { TextStyle::Value };
                    line.0.push((c, style));
                }
                // a non-active cursor standing at the line end (on the '\n' or at the
                // text end) has no char under it, show it on the padding space;
                // for a wrapped line (indexes.2) that position is the start of the
                // next line and will be shown there
                if !indexes.2 && edit.is_other_cursor(indexes.1) {
                    line.0.push((' ', TextStyle::EditCursor));
                }

                line.fix_length(width);
                lines.push(line);

                line_number_changed = !indexes.2; // increase line number after each '\n'
                if line_number_changed { line_number += 1 }
            }
        } else {
            if let Some(field_def) = root.get_field_definition(path) {
                line.add_field_name(field_def.name().clone(), indent, &cursor);

                // an open but unfocused editor shows its own text: it is committed
                // on focus loss, so normally it equals the field value in the data
                let mut value: Option<&String> = self.edit.as_ref().map(|edit| &edit.view.lines.text);
                if value.is_none() {
                    if let Some(field) = root.get_field(&path.0) {
                        if let FieldValue::SCALAR(ScalarValue::STR(data)) = &field.value {
                            value = Some(data);
                        }
                    }
                }

                if let Some(value) = value {
                    let line_by_line = self.get_lines_formated(width, indent, field_def.repeated(), amount == 0, value);
                    if line_by_line.len() <= 1 {
                        line.0.push((' ', TextStyle::Divider));
                        line.0.push(('\'', TextStyle::Divider));
                        line.add_string(value.to_string(), TextStyle::Value);
                        line.0.push(('\'', TextStyle::Divider));
                        line.fix_length(width);
                    } else { // multiline
                        let mut index = 0;
                        for text in line_by_line {
                            if index > 0 {
                                lines.push(line);
                                line = ScreenLine::new(width);
                                line.add_value_address(
                                    if text.1 {
                                        format!("{}", index + 1) // line after CR/LF
                                    } else {
                                        String::new() // line limited by length
                                    }, indent, &cursor, lines.len());
                            }
                            line.0.push((' ', TextStyle::Divider));
                            line.add_string(text.0.to_string(), TextStyle::Value);
                            line.fix_length(width);
                            if text.1 { index += 1 }
                        }
                    }
                } else {
                    line.0.push((' ', TextStyle::Divider));
                    line.0.push(('\'', TextStyle::Divider));
                    line.0.push(('\'', TextStyle::Divider));
                }
                lines.push(line);
                lines.first_mut().unwrap().add_typename(field_def, width, amount == 0);
            }
        }
        ScreenLines(lines)
    }

    fn get_text_edit_cursor(&self) -> Option<(u16, usize)> {
        if let Some(edit) = &self.edit {
            if let Some(pos) = edit.selected.get(edit.active_cursor_index)
            {
                let cfg = TextConfig::default();
                let res = edit.view.lines.to2d(&cfg, pos.pos);
                return Some((res.0 as u16, res.1));
            }
        }
        return None;
    }
    fn get_preferred_column(&self, _cursor_x: u16, _cursor_pos: usize) -> Option<u16> {
        self.edit.as_ref()
            .and_then(|edit| edit.selected.get(edit.active_cursor_index))
            .map(|sel| sel.x_pref as u16)
    }
    fn on_command(&mut self, root: &MessageData, path: &FieldPath, amount: usize, command: UserCommand, config: &LayoutConfig, width: u16, indent: u16, preferred_column: Option<u16>, cursor_x: &mut u16, cursor_pos: &mut usize) -> CommandResult
    {
        //        if let Some(field) = root.get_field(&path.0) {
        //            if let FieldValue::SCALAR(ScalarValue::STR(value)) = &field.value {
        //                self.visible_lines_count = self.get_lines_formated(width, indent, field.def.repeated(), value).len();
        //            }
        //        }
        //        if self.visible_lines_count < 1 { self.visible_lines_count = 1 }

        match command {
            UserCommand::ScrollVertically(mut delta) => {
                if let Some(edit) = &mut self.edit {
                    edit.on_move_y(&config.text_edit_cfg, delta, false);
                }
                *cursor_pos = (*cursor_pos as isize + delta) as usize;
                CommandResult::Redraw
            }

            UserCommand::ScrollHorizontally(delta) => {
                if let Some(edit) = &mut self.edit {
                    edit.on_move_x(&config.text_edit_cfg, delta as isize, false);
                } else {
                    if delta > 0 { // switch to edit mode from view mode
                        if let Some(field) = root.get_field(&path.0) {
                            if let FieldValue::SCALAR(ScalarValue::STR(value)) = &field.value {
                                let mut text_edit = TextEditor::new(value.clone(), width as usize, usize::MAX); // TODO
                                text_edit.on_move_y(&config.text_edit_cfg, *cursor_pos as isize, false);
                                self.edit = Some(text_edit);
                            }
                        }
                    }
                }
                CommandResult::Redraw
            }

            UserCommand::SelectHorizontally(delta) => {
                if let Some(edit) = &mut self.edit {
                    edit.on_move_x(&config.text_edit_cfg, delta as isize, true);
                    CommandResult::Redraw
                } else { CommandResult::None }
            }
            UserCommand::SelectHome => {
                if let Some(edit) = &mut self.edit {
                    edit.on_move_x(&config.text_edit_cfg, isize::MIN, true);
                    CommandResult::Redraw
                } else { CommandResult::None }
            }
            UserCommand::SelectEnd => {
                if let Some(edit) = &mut self.edit {
                    edit.on_move_x(&config.text_edit_cfg, isize::MAX, true);
                    CommandResult::Redraw
                } else { CommandResult::None }
            }
            UserCommand::SelectVertically(delta) => {
                if let Some(edit) = &mut self.edit {
                    edit.on_move_y(&config.text_edit_cfg, delta, true);
                    *cursor_pos = edit.cursor_line(&config.text_edit_cfg);
                    CommandResult::Redraw
                } else { CommandResult::None }
            }
            UserCommand::AddCursor(delta) => {
                if let Some(edit) = &mut self.edit {
                    edit.add_cursor(&config.text_edit_cfg, delta as isize);
                    *cursor_pos = edit.cursor_line(&config.text_edit_cfg);
                    CommandResult::Redraw
                } else { CommandResult::None }
            }
            UserCommand::Undo => {
                if let Some(edit) = &mut self.edit {
                    edit.undo(&config.text_edit_cfg);
                    *cursor_pos = edit.cursor_line(&config.text_edit_cfg);
                    CommandResult::Redraw
                } else { CommandResult::None }
            }
            UserCommand::Redo => {
                if let Some(edit) = &mut self.edit {
                    edit.redo(&config.text_edit_cfg);
                    *cursor_pos = edit.cursor_line(&config.text_edit_cfg);
                    CommandResult::Redraw
                } else { CommandResult::None }
            }
            UserCommand::FocusLost => { // the cursor moved to another field
                if let Some(edit) = &mut self.edit {
                    edit.clear_cursors();
                    // commit the typed text: the editor stays open in the background,
                    // but the data must not be lost when the document is saved or
                    // another editor is closed (both rebuild the layouts)
                    let mut modified = true;
                    if let Some(field) = root.get_field(&path.0) {
                        if let FieldValue::SCALAR(ScalarValue::STR(value)) = &field.value {
                            modified = *value != edit.view.lines.text;
                        }
                    }
                    if modified {
                        let new_field_value = FieldValue::SCALAR(ScalarValue::STR(edit.view.lines.text.clone()));
                        CommandResult::ChangeData(Change { path: path.clone(), action: ChangeType::Overwrite(new_field_value) })
                    } else { CommandResult::Redraw }
                } else { CommandResult::None }
            }
            UserCommand::FocusGained(from_below) => {
                if let (Some(edit), Some(column)) = (&mut self.edit, preferred_column) {
                    // continue the vertical movement started in the editor of a
                    // neighboring field at the same column
                    let y = if from_below { edit.view.lines.height(&config.text_edit_cfg) - 1 } else { 0 };
                    edit.set_cursor_at(&config.text_edit_cfg, column as usize, y);
                    *cursor_pos = edit.cursor_line(&config.text_edit_cfg);
                    CommandResult::Redraw
                } else { CommandResult::None }
            }

            UserCommand::DeleteData(backspace) => {
                if let Some(edit) = &mut self.edit {
                    edit.on_delete(&config.text_edit_cfg, backspace);
                    // deleting a selection or a '\n' can move the cursor to another line
                    *cursor_pos = edit.cursor_line(&config.text_edit_cfg);
                    CommandResult::Redraw
                } else {
                    on_command_default_handler(root, path, amount, command, config, width, indent, preferred_column, cursor_x, cursor_pos)
                    //CommandResult::None
                }
            }

            UserCommand::Home => {
                if let Some(edit) = &mut self.edit {
                    edit.on_move_x(&config.text_edit_cfg, isize::MIN, false);
                    CommandResult::Redraw
                } else { CommandResult::None }
            }
            UserCommand::End => {
                if let Some(edit) = &mut self.edit {
                    edit.on_move_x(&config.text_edit_cfg, isize::MAX, false);
                    CommandResult::Redraw
                } else { CommandResult::None }
            }
            UserCommand::KeyPress(c) => {
                if let Some(edit) = &mut self.edit {
                    edit.on_char(&config.text_edit_cfg, c);
                    CommandResult::Redraw
                } else { CommandResult::None }
            }

            UserCommand::CollapsedToggle => { // Enter key: split the line at the cursor
                if let Some(edit) = &mut self.edit {
                    edit.on_char(&config.text_edit_cfg, '\n');
                    *cursor_pos += 1; // the cursor now on the next screen line
                    CommandResult::Redraw
                } else { CommandResult::None }
            }

            UserCommand::Exit => { // on first press Esc exit editor, on the second close app
                if let Some(edit) = &mut self.edit {
                    if edit.selected.len() > 1 {
                        // in multi-cursor mode Esc first drops the additional cursors,
                        // the editor stays open with the active cursor only
                        edit.collapse_cursors();
                        CommandResult::Redraw
                    } else {
                        let new_field_value = FieldValue::SCALAR(ScalarValue::STR(edit.view.lines.text.clone()));
                        self.edit = None;
                        CommandResult::ChangeData(Change { path: path.clone(), action: ChangeType::Overwrite(new_field_value) })
                    }
                } else { CommandResult::Exit }
            }

            _ => on_command_default_handler(root, path, amount, command, config, width, indent, preferred_column, cursor_x, cursor_pos)
        }
    }
}

impl BytesLayout {
    fn calc_sizes_internal(&self, mut width: u16, indent: u16, repeated: bool, empty_field: bool) -> (usize, u16) {
        let mut free_width = width;
        free_width -= indent + 1; // field and ':'
        free_width -= 5; // "bytes".len()
        if empty_field { free_width -= 1 } // '-' before type name
        if repeated { free_width -= 1 } // '*' after type name

        let mut blocks_count = free_width / (8 * 3 + 1); // each block 8 bytes wide

        if blocks_count > 0 { // spaces between blocks
            free_width -= (blocks_count - 1);
            blocks_count = free_width / (8 * 3 + 1);
        }

        let bytes_on_line =
            if blocks_count == 0 {
                debug_assert!((free_width - 1) / 3 < 8);
                (free_width - 1) / 3
            } else {
                // if possible, concatenate the last short line with the first line
                if self.data_size as u16 > blocks_count * 8 {
                    let one_line_len = blocks_count * (8 * 3 + 1) + 1 + (self.data_size as u16 - blocks_count * 8) * 3;
                    if one_line_len <= free_width {
                        self.data_size as u16
                    } else { blocks_count * 8 }
                } else { blocks_count * 8 }
            }.max(1);

        // now we can calculate required number of lines
        let mut height = self.data_size / bytes_on_line as usize;
        if self.data_size != height * bytes_on_line as usize {
            height += 1;
        }
        height = height.max(1); // one line always shown, even if there is no data

        (height, bytes_on_line)
    }

    fn data_index_from_cursor(&self, cursor_x: u16, cursor_y: usize) -> Option<usize> {
        if cursor_x == 0 { None } else {
            Some(cursor_x as usize + self.bytes_per_line as usize * cursor_y - 1)
        }
    }

    // y: selected line in layout, x: selected byte (starting at 1) in line (not char)
    fn cursor_from_data_index(&self, index: usize) -> (u16, usize) {
        let y = index / self.bytes_per_line as usize;
        let x = index % self.bytes_per_line as usize;
        (x as u16 + 1, y)
    }

    fn change_current_byte(&mut self, root: &MessageData, path: &FieldPath, edit_pos: usize, byte_value: u8) -> CommandResult {
        if let Some(field) = root.get_field(&path.0) {
            if let FieldValue::SCALAR(BYTES(value)) = &field.value {
                let mut value = value.clone();
                if value.len() + 1 <= edit_pos {
                    value.push(byte_value);
                    self.data_size = value.len();
                } else {
                    value[edit_pos - 1] = byte_value;
                }
                return CommandResult::ChangeData(Change { path: path.clone(), action: ChangeType::Overwrite(FieldValue::SCALAR(BYTES(value))) });
            } else { debug_assert!(false) }
        } else { debug_assert!(false) }


        return CommandResult::Redraw;
    }
}

impl ViewLayout for BytesLayout {
    fn layout_type(&self) -> LayoutType { LayoutType::Bytes }
    fn calc_sizes(&mut self, root: &MessageData, path: &FieldPath, amount: usize, config: &LayoutConfig, width: u16, negotiator: &mut IndentsCalc) -> usize {

        // calculate width of first column as maximum length of field name and address
        let mut name_len = 0;
        let mut address_len = 0;
        self.data_size = 0;
        let mut repeated = false;
        if let Some(field) = root.get_field(&path.0) {
            debug_assert!(amount > 0);
            if let FieldValue::SCALAR(ScalarValue::BYTES(data)) = &field.value {
                self.data_size = data.len();
                address_len = format!("{:x}", self.data_size).len();
                name_len = field.def.name().len();
                repeated = field.def.repeated();
                debug_assert!(name_len > 0);
            }
        }
        if name_len == 0 { // no data was read, get field name from proto file
            if let Some(field_def) = root.get_field_definition(path) {
                name_len = field_def.name().len();
                repeated = field_def.repeated();
            }
        }
        let indent = negotiator.add(address_len.max(name_len), path.0.len());
        let (height, len) = self.calc_sizes_internal(width, indent, repeated, amount == 0);
        self.bytes_per_line = len;
        height
    }

    fn get_screen(&self, root: &MessageData, path: &FieldPath, amount: usize, width: u16, indent: u16, config: &LayoutConfig, cursor: Option<(u16, usize)>) -> ScreenLines {
        let mut lines = vec![];
        let mut line = ScreenLine::new(width);

        let selected_index = cursor.map_or(usize::MAX, |(x, y)| {
            self.data_index_from_cursor(x, y).unwrap_or(usize::MAX)
        });

        if let Some(field_def) = root.get_field_definition(path) {
            line.add_field_name(field_def.name().clone(), indent, &cursor);

            if let Some(field) = root.get_field(&path.0) {
                if let FieldValue::SCALAR(BYTES(value)) = &field.value {
                    for index in 0..value.len() {
                        if 0 != index {
                            if 0 == index % self.bytes_per_line as usize { // create new line
                                line.fix_length(width);
                                lines.push(line);
                                line = ScreenLine::new(width);
                                line.add_value_address(format!("{:X}", index), indent, &cursor, lines.len());
                            } else { // add space between every 8 bytes
                                if self.bytes_per_line > 8 && 0 == index & 7 { line.add_string(" ".to_string(), TextStyle::Value) }
                            }
                        }
                        let mut byte_value = value[index];
                        // show just entered by user data in editing mode
                        let style = if selected_index == index {
                            if let Some(edit) = &self.edit {
                                if let Some(nibble) = edit.first_nibble {
                                    byte_value = (nibble << 4) | (byte_value & 0x0f);
                                }
                            }
                            TextStyle::SelectedValue
                        } else { TextStyle::Value };
                        line.add_string(" ".to_string(), TextStyle::Divider);
                        line.add_string(format!("{:02X}", byte_value), style);
                    }
                }
            }
            line.fix_length(width);
            lines.push(line);
            lines.first_mut().unwrap().add_typename(field_def, width, amount == 0);
        }
        ScreenLines(lines)
    }

    fn get_text_edit_cursor(&self) -> Option<(u16, usize)> {
        if let Some(edit) = &self.edit {
            let (mut x, y) = self.cursor_from_data_index(edit.pos);
            x -= 1;
            let column_extra_spacing = x >> 3;
            x = 3 * x + column_extra_spacing;
            if edit.first_nibble.is_some() {
                x += 1;
            }
            Some((x, y))
        } else { None }
    }
    fn on_command(&mut self, root: &MessageData, path: &FieldPath, amount: usize, command: UserCommand, config: &LayoutConfig, width: u16, indent: u16, preferred_column: Option<u16>, cursor_x: &mut u16, cursor_pos: &mut usize) -> CommandResult {
        match command {
            UserCommand::DeleteData(_) => {
                if let Some(edit) = &mut self.edit {
                    let pos = edit.pos + 1;
                    return self.change_current_byte(root, path, pos, 0);
                } else {
                    if let Some(field) = root.get_field(&path.0) {
                        if let FieldValue::SCALAR(BYTES(value)) = &field.value {
                            if let Some(mut index) = self.data_index_from_cursor(*cursor_x, *cursor_pos) {
                                let mut value = value.clone();
                                index = index.min(value.len()-1);
                                value.remove(index);
                                self.data_size = value.len();
                                if self.data_size > 0 {
                                    (*cursor_x, *cursor_pos) = self.cursor_from_data_index(index.min(self.data_size - 1));
                                } else { *cursor_x = 0 }
                                return CommandResult::ChangeData(Change { path: path.clone(), action: ChangeType::Overwrite(FieldValue::SCALAR(BYTES(value))) });
                            }
                        }
                    }
                    self.edit = None;
                    on_command_default_handler(root, path, amount, command, config, width, indent, preferred_column, cursor_x, cursor_pos) // CommandResult::None
                }
            }

            UserCommand::InsertData => {
                if let Some(field) = root.get_field(&path.0) {
                    if let FieldValue::SCALAR(BYTES(value)) = &field.value {
                        if let Some(mut index) = self.data_index_from_cursor(*cursor_x, *cursor_pos) {
                            let mut value = value.clone();
                            index = index.min(value.len()-1);
                            value.insert(index + 1, 0);
                            self.data_size = value.len();
                            (*cursor_x, *cursor_pos) = self.cursor_from_data_index(index + 1);
                            return CommandResult::ChangeData(Change { path: path.clone(), action: ChangeType::Overwrite(FieldValue::SCALAR(BYTES(value))) });
                        }
                    }
                }
                CommandResult::None
            }

            UserCommand::KeyPress(c) => {
                if let Some(edit) = &mut self.edit {
                    let byte_changed = edit.on_char(c);
                    if let Some(edit) = &self.edit {
                        (*cursor_x, *cursor_pos) = self.cursor_from_data_index(edit.pos);
                        if let Some(byte_value) = byte_changed {
                            return self.change_current_byte(root, path, edit.pos, byte_value);
                        }
                    } else { debug_assert!(false) }
                    CommandResult::Redraw
                } else {
                    if let Some(index) = self.data_index_from_cursor(*cursor_x, *cursor_pos) {
                        let mut edit = ByteEditor::new(index);
                        edit.on_char(c);
                        self.edit = Some(edit);
                    }
                    CommandResult::Redraw
                }
            }

            UserCommand::ScrollHorizontally(delta) => {
                //debug_assert!(amount > 0);
                if delta > 0 {
                    if let Some(field) = root.get_field(&path.0) {
                        if let FieldValue::SCALAR(BYTES(value)) = &field.value {
                            if value.is_empty() {
                                self.edit = Some(ByteEditor::new(0));
                                return CommandResult::Redraw;
                            }
                        }
                    }
                    *cursor_x = (*cursor_x + delta as u16).min(self.bytes_per_line);
                    if *cursor_x as usize + *cursor_pos * self.bytes_per_line as usize > self.data_size {
                        *cursor_x = (self.data_size % self.bytes_per_line as usize) as u16;
                    }
                } else { // delta < 0
                    let delta = (-delta as u16).min(*cursor_x);
                    *cursor_x -= delta;
                }
                self.edit = None;
                CommandResult::Redraw
            }

            UserCommand::Home => {
                *cursor_x = if *cursor_x == 1 { 0 } else { 1 };
                self.edit = None;
                CommandResult::Redraw
            }

            UserCommand::End => {
                *cursor_x = self.bytes_per_line;
                let index = self.data_index_from_cursor((*cursor_x).max(1), *cursor_pos).unwrap();
                (*cursor_x, *cursor_pos) = self.cursor_from_data_index(index.min(self.data_size - 1));
                self.edit = None;
                CommandResult::Redraw
            }

            UserCommand::Exit => { // on first press Esc exit editor, on the second close app
                if let Some(edit) = &mut self.edit {
                    self.edit = None;
                    CommandResult::Redraw
                } else { CommandResult::Exit }
            }

            _ => on_command_default_handler(root, path, amount, command, config, width, indent, preferred_column, cursor_x, cursor_pos)
        }
    }

    fn get_consumed_fields(&self, root: &MessageData, path: &FieldPath, config: &LayoutConfig) -> HashSet<i32> {
        todo!()
    }

    fn get_status_string(&self, cursor_x: u16, cursor_y: usize) -> String {
        self.data_index_from_cursor(cursor_x, cursor_y).map_or(String::new(), |index| format!("{}/{}", index, self.data_size))
    }
}

impl MessageLayout {
    fn new() -> Self {
        MessageLayout { scroll: 0 }
    }
}
impl ViewLayout for MessageLayout {
    fn layout_type(&self) -> LayoutType { LayoutType::Message }
    fn calc_sizes(&mut self, root: &MessageData, path: &FieldPath, amount: usize, config: &LayoutConfig, width: u16, negotiator: &mut IndentsCalc) -> usize {
        if let Some(field_def) = root.get_field_definition(path) {
            negotiator.add(field_def.name().len(), path.0.len());
        }
        return 1;
    }
    fn get_screen(&self, root: &MessageData, path: &FieldPath, amount: usize, width: u16, indent: u16, config: &LayoutConfig, cursor: Option<(u16, usize)>) -> ScreenLines {
        debug_assert!(amount <= 1);
        let mut line = ScreenLine::new(width);
        if let Some(field_def) = root.get_field_definition(path) {
            line.add_field_name(field_def.name().clone(), indent, &cursor);
            line.add_typename(field_def, width, amount == 0);
        }
        ScreenLines(vec![line])
    }
    fn on_command(&mut self, root: &MessageData, path: &FieldPath, amount: usize, command: UserCommand, config: &LayoutConfig, width: u16, indent: u16, preferred_column: Option<u16>, cursor_x: &mut u16, cursor_pos: &mut usize) -> CommandResult
    {
        match command {
            //UserCommand::TableTreeToggle => { CommandResult::ChangeLayout(LayoutType::Table) }
            _ => on_command_default_handler(root, path, amount, command, config, width, indent, preferred_column, cursor_x, cursor_pos)
        }
    }
}

impl TableLayout {
    fn new(path: FieldPath) -> Self {
        TableLayout { vertical: false, scroll: (0, 0) }
    }
}
impl ViewLayout for TableLayout {
    fn layout_type(&self) -> LayoutType { LayoutType::Table }
    fn calc_sizes(&mut self, root: &MessageData, path: &FieldPath, amount: usize, config: &LayoutConfig, width: u16, negotiator: &mut IndentsCalc) -> usize {
        todo!()
    }
    fn get_screen(&self, root: &MessageData, path: &FieldPath, amount: usize, width: u16, indent: u16, config: &LayoutConfig, cursor: Option<(u16, usize)>) -> ScreenLines {
        let mut line = ScreenLine::new(width);
        if let Some(field) = root.get_field(&path.0) {
            line.add_field_name(field.def.name().clone(), indent, &cursor);
            line.add_typename(field.def.clone(), width, amount == 0);
        }
        ScreenLines(vec![line])
    }
    fn on_command(&mut self, root: &MessageData, path: &FieldPath, amount: usize, command: UserCommand, config: &LayoutConfig, width: u16, indent: u16, preferred_column: Option<u16>, cursor_x: &mut u16, cursor_pos: &mut usize) -> CommandResult
    {
        match command {
            _ => on_command_default_handler(root, path, amount, command, config, width, indent, preferred_column, cursor_x, cursor_pos)
        }
    }
}

impl ViewLayout for CollapsedLayout {
    fn layout_type(&self) -> LayoutType { LayoutType::Collapsed }
    fn calc_sizes(&mut self, root: &MessageData, path: &FieldPath, amount: usize, config: &LayoutConfig, width: u16, negotiator: &mut IndentsCalc) -> usize {
        let def = root.get_field_definition(path).unwrap();
        negotiator.add(def.name().len(), path.0.len());
        return 1;
    }
    fn get_screen(&self, root: &MessageData, path: &FieldPath, amount: usize, width: u16, indent: u16, config: &LayoutConfig, cursor: Option<(u16, usize)>) -> ScreenLines {
        let mut line = ScreenLine::new(width);

        if let Some(field_def) = root.get_field_definition(path) {
            line.add_field_name(field_def.name().clone(), indent, &cursor);
            line.add_field_size(self.display_size, width);
            line.add_typename(field_def.clone(), width, self.display_size == 0);
        }


        //        if let Some(field) = root.get_field(&path.0) {
        //            line.add_field_name(field.def.name().clone(), indent, &cursor);
        //            line.add_field_size(self.display_size, width);
        //            line.add_typename(field.def.clone(), width, self.display_size == 0);
        //        }
        ScreenLines(vec![line])
    }
    fn on_command(&mut self, root: &MessageData, path: &FieldPath, amount: usize, command: UserCommand, config: &LayoutConfig, width: u16, indent: u16, preferred_column: Option<u16>, cursor_x: &mut u16, cursor_pos: &mut usize) -> CommandResult {
        match command {
            _ => on_command_default_handler(root, path, amount, command, config, width, indent, preferred_column, cursor_x, cursor_pos)
        }
    }

    fn get_consumed_fields(&self, root: &MessageData, path: &FieldPath, config: &LayoutConfig) -> HashSet<i32> {
        if let Some(msg) = root.get_submessage(&path.0) {
            return msg.def.fields.iter().map(|field| field.id()).collect();
        }
        unreachable!()
    }
}

impl TextStyle {
    pub fn first_column(&self) -> bool {
        match self {
            TextStyle::FieldName |
            TextStyle::FieldIndex |
            TextStyle::SelectedFieldIndex |
            TextStyle::SelectedFieldName => true,
            _ => false,
        }
    }

    pub fn activate(&self) -> impl crossterm::Command {

        // color theme may use 16 color, 256 color or true color mode,
        // different modes compatible with different terminals

        let foreground_color = match self {
            TextStyle::TopLine => Color::Black,
            TextStyle::FieldName => Color::Green,
            TextStyle::SelectedValue |
            TextStyle::SelectedFieldIndex |
            TextStyle::SelectedFieldName |
            TextStyle::EditCursor => Color::Black,
            TextStyle::FieldIndex |
            TextStyle::Divider => Color::DarkGrey,
            TextStyle::Value => Color::White, // Color::AnsiValue(230), // https://www.ditig.com/256-colors-cheat-sheet
            TextStyle::DefaultValue => Color::Grey,
            TextStyle::Typename => Color::DarkCyan,
            TextStyle::Bookmark => Color::Black,
            TextStyle::Unknown => Color::Reset,
            _ => Color::Grey,
        };

        let background_color = match self {
            TextStyle::TopLine => Color::DarkCyan,
            TextStyle::EditCursor => Color::Grey, // static reverse-video cell, does not blink
            TextStyle::SelectedValue |
            TextStyle::SelectedFieldName |
            TextStyle::SelectedFieldIndex |
            TextStyle::SelectedTypename => Color::DarkCyan,
            TextStyle::Bookmark => Color::Yellow,
            _ => Color::Reset,
        };

        style::SetColors(style::Colors {
            foreground: Some(foreground_color),
            background: Some(background_color),
        })
    }
}

impl LayoutParams {
    pub fn new(path: FieldPath, amount: usize, layout: Box<dyn ViewLayout>) -> LayoutParams {
        LayoutParams { height: 1, path, amount, layout: Some(layout), children_count: 0 }
    }
    pub fn new_empty(path: FieldPath, amount: usize) -> LayoutParams {
        LayoutParams { height: 1, path, amount, layout: None, children_count: 0 }
    }
    pub fn level(&self) -> usize {
        self.path.0.len()
    }
    pub fn get_status_string(&self, cursor_x: u16, cursor_y: usize) -> String {
        if let Some(layout) = self.layout.as_ref() {
            return layout.get_status_string(cursor_x, cursor_y);
        }
        String::new()
    }
    pub fn calc_sizes(&mut self, root: &MessageData, config: &LayoutConfig, width: u16, negotiator: &mut IndentsCalc) {
        if let Some(layout) = &mut self.layout {
            self.height = layout.as_mut().calc_sizes(root, &self.path, self.amount, config, width, negotiator);
        }
    }

    pub fn get_screen(&self, root: &MessageData, width: u16, indent: u16, config: &LayoutConfig, cursor: Option<(u16, usize)>) -> ScreenLines
    {
        if let Some(layout) = &self.layout {
            layout.get_screen(root, &self.path, self.amount, width, indent, config, cursor)
        } else {
            debug_assert!(false);
            ScreenLines::new()
        }
    }

    pub fn get_text_edit_cursor(&self) -> Option<(u16, usize)> {
        if let Some(layout) = &self.layout {
            layout.get_text_edit_cursor()
        } else {
            None
        }
    }

    pub fn get_preferred_column(&self, cursor_x: u16, cursor_pos: usize) -> Option<u16> {
        if let Some(layout) = &self.layout {
            layout.get_preferred_column(cursor_x, cursor_pos)
        } else {
            None
        }
    }

    pub fn on_command(&mut self, root: &MessageData, command: UserCommand, config: &LayoutConfig, width: u16, indent: u16, preferred_column: Option<u16>, cursor_x: &mut u16, cursor_pos: &mut usize) -> CommandResult {
        if let Some(layout) = &mut self.layout {
            match command {
                _ => layout.on_command(root, &self.path, self.amount, command, config, width, indent, preferred_column, cursor_x, cursor_pos),
            }
        } else { CommandResult::None }
    }
}

impl Layouts {
    pub fn new(root: &MessageData, proto: ProtoData, config: &LayoutConfig, file_path: std::path::PathBuf, width: u16, height: u16) -> Layouts {
        let sorted_fields = root.get_sorted_fields(&config.field_order);
        let mut items: Vec<LayoutParams> =
            sorted_fields.into_iter().enumerate().
                map(|(layout_index, pos_ex)| Self::create_field_layouts(root, &config, &FieldPath([pos_ex.0].into()), pos_ex.1, false)).
                flatten().collect();

        let mut negotiator = IndentsCalc::new();

        for item in &mut items {
            item.calc_sizes(root, config, width, &mut negotiator); // for scalar field only, messages are empty
        }

        let top_layouts_count = Self::calc_top_layouts_count(&items);

        Layouts { items, proto, file_path, indents: negotiator.level_indents, scroll: 0, top_layouts_count, width, height, delimited: false }
    }

    pub fn file_name(&self) -> String {
        self.file_path.file_name().unwrap().to_string_lossy().into_owned()
    }

    pub fn save_document(&self, data: &MessageData) -> io::Result<bool> {
        let mut temp_path = self.file_path.clone();
        temp_path.set_extension("tmp");
        {
            let mut output = std::fs::File::create(temp_path.clone())?;
            if self.delimited {
                data.write_delimited(&mut output, &self.proto)?;
            } else {
                data.write(&mut output, &self.proto, data.def.clone())?;
            }
            output.flush()?;
        }
        std::fs::rename(temp_path, self.file_path.clone())?;
        Ok(false)
    }


    fn create_field_layouts(root: &MessageData, config: &LayoutConfig, path: &FieldPath, amount: usize, load_all: bool) -> Vec<LayoutParams> {
        let mut items: Vec<LayoutParams> = vec![];
        let last_pos = path.0.last().unwrap().clone();
        if let Some(field) = root.get_field(&path.0) {
            match &field.value {
                FieldValue::MESSAGE(msg) => {
                    if amount == 0 {
                        items.append(&mut Self::create_message_layouts(root, config, path, amount, load_all));
                    } else {
                        for index in last_pos.index..last_pos.index + amount { // message layout does not support repeated data
                            items.append(&mut Self::create_message_layouts(root, config, &path.with_last_index(index), 1, load_all));
                        }
                    }
                }
                FieldValue::SCALAR(scalar) => {
                    items.append(&mut Self::create_scalar_layouts(field.def.clone(), path.clone(), amount));
                }
            }
        } else { // no data was read, show empty field
            let field_def = root.get_field_definition(&path).unwrap();
            debug_assert!(amount == 0);
            if field_def.is_message() {
                items.append(&mut Self::create_message_layouts(root, config, path, amount, load_all));
            } else {
                items.append(&mut Self::create_scalar_layouts(field_def, path.clone(), amount));
            }
        }
        items
    }

    pub fn create_message_layouts(root: &MessageData, config: &LayoutConfig, path: &FieldPath, amount: usize, load_all: bool) -> Vec<LayoutParams> {
        let mut items: Vec<LayoutParams> = vec![];
        if load_all {
            let msg_layout = MessageLayout::new();
            let consumed_fields = msg_layout.get_consumed_fields(root, path, config);
            items.push(LayoutParams::new(path.clone(), amount, Box::new(msg_layout)));
            if amount > 0 {
                let msg = root.get_submessage(&path.0).unwrap();
                let sorted_fields = msg.get_sorted_fields(&config.field_order);
                let mut descendants = sorted_fields.into_iter().
                    filter(|(pos, _)| !consumed_fields.contains(&pos.id)).
                    map(|(pos, amount)| Self::create_field_layouts(root, config, &path.add(pos), amount, load_all)).
                    flatten().collect::<Vec<LayoutParams>>();
                items.last_mut().unwrap().children_count = Self::calc_top_layouts_count(&descendants);
                items.append(&mut descendants);
            }
        } else {
            items.push(LayoutParams::new_empty(path.clone(), amount));
        }
        items
    }

    fn create_scalar_layouts(field_def: FieldProtoPtr, path: FieldPath, amount: usize) -> Vec<LayoutParams> {
        let mut items: Vec<LayoutParams> = vec![];
        let layout_type = &field_def.typename();
        match layout_type.as_str() {
            // repeated strings and bytes always shown as one layout for each data item
            "bytes" | "string" => {
                let start = path.0.last().unwrap().index;
                for index in start..start + amount.max(1) {
                    let layout: Box<dyn ViewLayout> = if layout_type.as_str() == "bytes" {
                        Box::new(BytesLayout {
                            bytes_per_line: 0,
                            data_size: 0,
                            edit: None,
                        })
                    } else {
                        Box::new(StringLayout {
                            edit: None,
                            visible_lines_count: 0,
                        })
                    };
                    items.push(LayoutParams::new(path.with_last_index(index), amount.min(1), layout))
                }
            }
            _ => items.push(LayoutParams::new(path, amount, Box::new(ScalarLayout::new()))),
        }
        items
    }

    pub fn start_indent_update(&mut self) -> IndentsCalc {
        let indents = mem::replace(&mut self.indents, vec![]);
        IndentsCalc::new_for_update(indents)
    }

    pub fn update_layouts(&mut self, root: &MessageData, config: &LayoutConfig) {
        let mut negotiator = self.start_indent_update();
        for item in &mut self.items {
            item.calc_sizes(root, config, self.width, &mut negotiator);
        }
        self.indents = negotiator.into();
    }


    pub fn ensure_loaded(&mut self, root: &MessageData, config: &LayoutConfig, layout_index: usize, lines_before: usize, lines_after: usize, selection: &mut Selection) {
        let mut remain = lines_after as isize;
        let mut i = layout_index;
        while i < self.items.len() {
            //
            if self.items[i].layout.is_some() {
                let mut indent_calc = self.start_indent_update();
                let item = &mut self.items[i];
                item.calc_sizes(root, config, self.width, &mut indent_calc);
                self.indents = indent_calc.into();
                remain -= item.height as isize;
                i += 1;
            } else {
                let (count, lines_count) = self.expand_collapsed(root, config, i);
                remain -= lines_count as isize;
                i += count;
            }
            if remain <= 0 { break; }
        }

        remain = lines_before as isize;
        let mut i = layout_index;
        while i > 0 {
            i -= 1; // [i=0] already processed above
            if self.items[i].layout.is_some() {
                let mut indent_calc = self.start_indent_update();
                let item = &mut self.items[i];
                item.calc_sizes(root, config, self.width, &mut indent_calc);
                self.indents = indent_calc.into();
                remain -= item.height as isize;
            } else {
                let (count, lines_count) = self.expand_collapsed(root, config, i);
                remain -= lines_count as isize;
                if selection.layout > i {
                    selection.layout += count;
                }
            }
            if remain <= 0 { break; }
        }
    }

    // how many layouts in the vector has minimal available level
    fn calc_top_layouts_count(items: &Vec<LayoutParams>) -> usize {
        let mut level = None;
        let mut top_level_count = 0;

        for layout in items {
            if let Some(level) = level {
                if layout.level() == level {
                    top_level_count += 1;
                }
                debug_assert!(layout.level() >= level);
            } else {
                level = Some(layout.level());
                top_level_count = 1;
            }
        }
        top_level_count
    }

    fn calc_children_count(&self, parent_pos: usize) -> usize {
        if let Some(current) = self.items.get(parent_pos) {
            let path_len = current.path.0.len();
            let mut end_pos = parent_pos + 1;
            while end_pos < self.items.len() {
                let len = self.items[end_pos].path.0.len();
                if len <= path_len { break; }
                end_pos += 1;
            }
            return end_pos - parent_pos;
        }
        0
    }


    // restore message layout with children
    // return a new count of layouts (instead of 1 before) and total lines in them
    fn expand_collapsed(&mut self, root: &MessageData, config: &LayoutConfig, pos: usize) -> (usize, usize) {
        let mut new_layout_count = 0;
        let mut new_lines_count = 0;
        let mut path = None;
        if let Some(current) = self.items.get(pos) {
            path = Some(current.path.clone());
        }
        if let Some(path) = path {
            let mut negotiator = self.start_indent_update();
            let amount = if root.get_field(&path.0).is_some() { 1 } else { 0 };
            let mut layouts = Self::create_message_layouts(root, config, &path, amount, true);
            new_layout_count = layouts.len();
            self.items.remove(pos);
            while !layouts.is_empty() {
                let mut new_item = layouts.pop().unwrap();
                new_item.calc_sizes(root, config, self.width, &mut negotiator);
                new_lines_count += new_item.height;
                self.items.insert(pos, new_item);
            }
            self.indents = negotiator.into();
        }
        debug_assert!(new_layout_count > 0);
        debug_assert!(new_lines_count > 0);
        (new_layout_count, new_lines_count)
    }


    pub fn calc_relative_pos(&self, mut pos: usize) -> f32 {
        let mut index = 0;
        let mut level = usize::MAX;

        // index of a layout and count of its sibling
        // the index started at zero for top-level layouts and at one for nested layouts
        let mut index_n_size = Vec::with_capacity(16);
        loop {
            if let Some(layout) = self.items.get(pos) {
                let current_level = layout.level();
                if current_level < level {
                    debug_assert!(level == current_level + 1 || level == usize::MAX);
                    level = current_level;
                    index_n_size.push(index);
                    index_n_size.push(layout.children_count);
                    index = 0;
                }

                if pos == 0 {
                    index_n_size.push(index);
                    break;
                }
                pos -= 1;
                if current_level == level { index += 1; }
            } else {
                return if self.items.is_empty() { 0.0 } else { 1.0 };
            }
        }
        index_n_size.push(self.top_layouts_count);
        debug_assert!(index_n_size.len() & 1 == 0);

        let mut position = 0.0;
        let mut valuable = 1.0;
        while !index_n_size.is_empty() {
            let size = index_n_size.pop().unwrap();
            let index = index_n_size.pop().unwrap();

            if size > 0 {
                position += valuable * (index as f32 / size as f32);
                valuable = valuable / ((size + 1) as f32);
            }
        }

        position
    }

    pub fn get_parent_pos(&self, mut pos: usize) -> Option<usize> {
        if let Some(current) = self.items.get(pos) {
            let parent_len = current.path.0.len() - 1;
            while pos > 0 {
                pos -= 1;
                let len = self.items[pos].path.0.len();
                if len == parent_len { return Some(pos); }
            }
        }
        None
    }

    pub fn update_after_data_changed(&mut self, root: &MessageData, config: &LayoutConfig, changed_layout: usize) {
        // take out the layouts with an open in-place editor: the rebuild below
        // replaces the layout objects, but an open editor (its text, cursor state
        // and undo history) must survive a data change of a neighboring field;
        // they are matched back to the new layouts by field path
        let mut open_editors: Vec<(Vec<FieldPos>, Box<dyn ViewLayout>)> = vec![];
        for item in &mut self.items {
            if item.get_text_edit_cursor().is_some() {
                if let Some(layout) = item.layout.take() {
                    open_editors.push((item.path.0.clone(), layout));
                }
            }
        }

        let mut negotiator = self.start_indent_update();

        // when a field changed, recreate layout of the parent message.
        // the field may be repeated, so delete/create it may influence siblings
        if let Some(parent_pos) = self.get_parent_pos(changed_layout) {
            let children_count = self.calc_children_count(parent_pos);
            if let Some(parent) = self.items.get_mut(parent_pos) {
                parent.children_count = children_count;
            }

            if let Some(parent) = self.items.get(parent_pos) {
                if let Some(parent_msg) = root.get_submessage(&parent.path.0) {
                    let mut layouts = Self::create_message_layouts(root, config, &parent.path, 1, true);
                    self.items.drain(parent_pos..parent_pos + children_count);
                    while !layouts.is_empty() {
                        let mut new_item = layouts.pop().unwrap();
                        new_item.calc_sizes(root, config, self.width, &mut negotiator);
                        self.items.insert(parent_pos, new_item);
                    }
                }
            }
        } else { // if changed a field of the root message, rebuild all layouts
            let sorted_fields = root.get_sorted_fields(&config.field_order);
            let mut items: Vec<LayoutParams> =
                sorted_fields.into_iter().
                    map(|pos_ex| Self::create_field_layouts(root, &config, &FieldPath([pos_ex.0].into()), pos_ex.1, true)).
                    flatten().collect();

            for item in &mut items {
                item.calc_sizes(root, config, self.width, &mut negotiator);
            }
            self.top_layouts_count = Self::calc_top_layouts_count(&items);
            self.items = items;
        }

        // put the editors back into the layouts of the same fields
        // (an editor of a field that disappeared is dropped)
        for (path, layout) in open_editors {
            if let Some(item) = self.items.iter_mut().find(|item| item.path.0 == path) {
                item.layout = Some(layout);
            }
        }
        self.indents = negotiator.into();
    }

    fn run_active_layout_command(&mut self, command: UserCommand, root: &MessageData, config: &LayoutConfig, selection: &mut Selection) -> CommandResult
    {
        if let Some(current) = self.items.get(selection.layout) {
            if let Some(&indent) = self.indents.get(current.level() - 1) {
                return self.items[selection.layout].on_command(root, command, config, self.width, indent, selection.preferred_column, &mut selection.x, &mut selection.y);
            } else { debug_assert!(false); }
        } else { debug_assert!(self.items.is_empty()); }
        CommandResult::None
    }

    // the selected layout is changing: tell the old one to drop its editor cursors
    // and selection (an inactive editor stores no cursor information).
    // An editor with modified text answers with ChangeData committing the text —
    // the caller must return it to the App
    fn notify_focus_lost(&mut self, root: &MessageData, config: &LayoutConfig, old_layout: usize) -> CommandResult {
        let Some(item) = self.items.get(old_layout) else { return CommandResult::None; };
        let Some(&indent) = self.indents.get(item.level() - 1) else { debug_assert!(false); return CommandResult::None; };
        // the cursor coordinates are not meaningful for an unselected layout
        let (mut unused_x, mut unused_y) = (0u16, 0usize);
        self.items[old_layout].on_command(root, UserCommand::FocusLost, config, self.width, indent, None, &mut unused_x, &mut unused_y)
    }

    pub fn run_command(&mut self, command: UserCommand, root: &MessageData, config: &LayoutConfig, selection: &mut Selection) -> CommandResult {
        let vertical_move = matches!(command,
            UserCommand::ScrollVertically(_) | UserCommand::ScrollSibling(_) | UserCommand::ScrollToBottom);

        let result = match &command {
            UserCommand::ScrollVertically(mut delta) => {
                let old_layout = selection.layout;
                let moving_up = delta < 0;
                let mut from_beneath = false;
                let mut move_in_layout = false; // the movement continues inside the reached layout

                while delta != 0 {
                    if let Some(current) = self.items.get(selection.layout) {
                        debug_assert!(current.layout.is_some());
                        if delta > 0 { // cursor moving down
                            if (selection.y + delta as usize) < current.height {
                                debug_assert!(selection.y < current.height);
                                move_in_layout = true;
                                break;
                            }
                            delta -= current.height as isize - selection.y as isize;

                            if selection.layout + 1 >= self.items.len() {
                                selection.y = current.height - 1;
                                break;
                            }
                            selection.layout += 1;
                            selection.y = 0;
                        } else { // cursor moving up

                            if from_beneath { selection.y = current.height - 1; }

                            if selection.y >= -delta as usize {
                                move_in_layout = true;
                                break;
                            }
                            delta += (selection.y + 1) as isize;

                            if selection.layout == 0 {
                                selection.y = 0;
                                break;
                            }
                            selection.layout -= 1;
                            from_beneath = true;
                        }
                    } else { // if no layouts exists
                        *selection = Selection::default();
                        break;
                    }
                }

                if selection.layout != old_layout {
                    let focus_result = self.notify_focus_lost(root, config, old_layout);
                    // the cursor continues at the remembered column. A layout that
                    // cannot take it (a structure name, or no column is remembered
                    // because the name column is traveling) ignores the command:
                    // its name gets selected instead, and the remembered column
                    // stays for the moves that follow
                    if let CommandResult::None = self.run_active_layout_command(UserCommand::FocusGained(moving_up), root, config, selection) {
                        selection.x = 0;
                    }
                    // the unfocused editor commits its modified text; the App applies
                    // it and rebuilds the layouts (open editors survive the rebuild,
                    // see update_after_data_changed)
                    if let CommandResult::ChangeData(change) = focus_result {
                        return CommandResult::ChangeData(change);
                    }
                    return CommandResult::Redraw;
                }
                if move_in_layout {
                    return self.run_active_layout_command(command, root, config, selection);
                }
                CommandResult::Redraw
            }

            UserCommand::ScrollSibling(delta) => {
                let old_layout = selection.layout;
                self.scroll_sibling(*delta, selection);
                if selection.layout != old_layout {
                    if let CommandResult::ChangeData(change) = self.notify_focus_lost(root, config, old_layout) {
                        return CommandResult::ChangeData(change);
                    }
                }
                CommandResult::Redraw
            }

            UserCommand::ScrollToBottom => {
                self.ensure_loaded(root, config, self.items.len() - 1, (2 * self.height + 1) as usize, 0, selection);
                let old_layout = selection.layout;
                selection.layout = self.items.len() - 1;
                selection.y = self.items[selection.layout].height - 1;
                selection.x = 0;
                if selection.layout != old_layout {
                    if let CommandResult::ChangeData(change) = self.notify_focus_lost(root, config, old_layout) {
                        return CommandResult::ChangeData(change);
                    }
                }
                CommandResult::Redraw
            }

            UserCommand::InsertData => {
                if selection.x == 0 && selection.y == 0 {
                    if let Some(current) = self.items.get(selection.layout) {
                        let def = root.get_field_definition(&current.path).unwrap();
                        CommandResult::ChangeData(Change { path: current.path.clone(), action: ChangeType::Insert(def.default()) })
                    } else { CommandResult::None }
                } else {
                    self.run_active_layout_command(command, root, config, selection)
                }
            }

            UserCommand::CollapsedToggle => {
                let layout_type = self.items.get(selection.layout)
                    .and_then(|current| current.layout.as_ref())
                    .map(|layout| layout.layout_type());
                match layout_type {
                    Some(LayoutType::Message) => {
                        let current = &self.items[selection.layout];
                        let current_path = current.path.clone();
                        let current_amount = current.amount;
                        // there is no reason to collapse a message that does not exist, it's already displayed in one line
                        if let Some(msg) = root.get_submessage(&current_path.0) {
                            // remove selected layout and all nested layouts
                            let path_len = current_path.0.len();
                            let mut end_pos = selection.layout + 1;
                            while end_pos < self.items.len() {
                                let len = self.items[end_pos].path.0.len();
                                if len <= path_len { break; }
                                end_pos += 1;
                            }
                            self.items.drain(selection.layout + 1..end_pos);
                            // create a collapsed layout in place of the deleted
                            self.items[selection.layout] = LayoutParams::new(current_path, current_amount, Box::new(CollapsedLayout { display_size: msg.len() }));
                        }
                        CommandResult::Redraw
                    }
                    Some(LayoutType::Collapsed) => {
                        self.expand_collapsed(root, config, selection.layout);
                        CommandResult::Redraw
                    }
                    // other layouts may consume Enter themselves (e.g. new line in a string editor)
                    Some(_) => self.run_active_layout_command(command, root, config, selection),
                    None => CommandResult::Redraw,
                }
            }
            _ => self.run_active_layout_command(command, root, config, selection)
        };

        // vim-style sticky column: any non-vertical command positions the cursor
        // explicitly, so its screen column becomes the target the vertical moves
        // aim at; the vertical moves themselves never change it, so a clamp on a
        // short line does not lose the target. None means the name column (x = 0)
        // is traveling instead of a value column
        if !vertical_move {
            selection.preferred_column = self.items.get(selection.layout).and_then(|item| item.get_preferred_column(selection.x, selection.y));
        }
        result
    }

    pub fn scroll_sibling(&self, delta: i8, selection: &mut Selection) -> bool {
        assert!(delta == -1 || delta == 1);
        let mut it = self.items.iter();
        let mut pos = 0;
        loop {
            if let Some(item) = it.next() {
                if pos == selection.layout {
                    let level = item.level();
                    loop {
                        let new_pos = pos as isize + delta as isize;
                        if new_pos < 0 || new_pos >= self.items.len() as isize { break; }
                        pos = new_pos as usize;


                        let item_level = self.items[pos].path.0.len();
                        if item_level < level {
                            return false;
                        }
                        if item_level == level {
                            selection.layout = pos;
                            selection.y = 0;
                            return true;
                        }
                    }
                    break;
                }
                pos += 1;
            } else { break; }
        }
        false // sibling field not found
    }
}

impl Debug for ScreenLine {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut first = true;
        for item in &self.0 {
            write!(f, "{}", item.0)?;
        }
        Ok(())
    }
}

impl Debug for ScreenLines {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for line in &self.0 {
            writeln!(f, "{:?}", line)?;
        }
        Ok(())
    }
}
