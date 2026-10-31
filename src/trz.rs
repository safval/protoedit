// TODO    pub fn change<'y>(&self) -> Changes<'y> where 'x: 'y
// { Changes { old: vec![], new: vec![], root_message: self } }

use std::path::PathBuf;
use crate::wire::{FieldPath, FieldValue, MessageData, ScalarValue};


pub struct Change {
    pub path: FieldPath,
    pub action: ChangeType,
}
pub enum ChangeType {
    Overwrite(FieldValue), // overwrite field data, old value for undo or new for redo
    Insert(FieldValue),    // insert new field
    Delete,                // remove field
    Batch(Vec<Change>),    // several changes applied in order, e.g. a oneof switch
}

pub struct History {
    pub undo: Vec<Change>,
    pub redo: Vec<Change>,
}

impl Change {
    pub fn change_value(path: FieldPath, value: ScalarValue) -> Self { Self { path, action: ChangeType::Overwrite(FieldValue::SCALAR(value)) } }
    pub fn insert_scalar(path: FieldPath, value: ScalarValue) -> Self { Self { path, action: ChangeType::Insert(FieldValue::SCALAR(value)) } }
    pub fn insert_message(path: FieldPath, value: MessageData) -> Self { Self { path, action: ChangeType::Insert(FieldValue::MESSAGE(value)) } }
    pub fn delete_value(path: FieldPath) -> Self { Self { path, action: ChangeType::Delete } }
    // switch a oneof: delete its set members, then insert the new one
    pub fn oneof_switch(deleted: Vec<FieldPath>, path: FieldPath, value: FieldValue) -> Self {
        let mut changes: Vec<Change> = deleted.into_iter().map(Self::delete_value).collect();
        changes.push(Self { path: path.clone(), action: ChangeType::Insert(value) });
        Self { path, action: ChangeType::Batch(changes) }
    }
    pub fn layout_changed(&self) -> bool {
        match self.action {
            ChangeType::Insert(_) => true,
            ChangeType::Delete => true,
            ChangeType::Batch(_) => true,
            ChangeType::Overwrite(_) => false,
        }
    }

}