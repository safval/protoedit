# protoedit

Terminal-based [protobuf](https://protobuf.dev) data files editor.

Fields missing from the .proto file, for example when the file is written by a newer
version of the format, and fields of types not found in it (e.g. a missing import) are
shown as raw bytes and kept as they are, so a file saved without changes is
byte-identical to the original.

## Command Line Interface

`protoedit data.pb;format.proto;message_name`

 * data.pb - path to file in protobuf format
 * format.proto - path to .proto file with data description (optional, by default
   the data file name with a `.proto` extension)
 * message_name - name of the root message in .proto (optional, by default the message
   not used as a field of any other one)

Options:

 * `-d`/`--delimited` - the data file is a stream of length-delimited records, the framing
   written by `writeDelimitedTo` (Java) or `pb_encode_delimited` (nanopb), instead of a
   single message. The records are shown as one repeated field, saving writes the prefixes back.
 * `-I`/`--proto_path <dir>` - a directory where the `import`ed proto files are searched,
   the option can be repeated (absolute paths only). The imported definitions are merged
   into the main proto file, the root message is searched only in the main file.
 * `--help`, `--version`

## Editing

To edit a scalar field, type a value over it, or press Enter to edit the current one.
To edit a string or bytes field, press the Right arrow on it. Esc or moving to another
field commits the edit. A field missing from the data is shown with its default value and
a minus before the type name; editing it or pressing Ins on its name creates the field.

To change an enum value, press Enter on it and choose the new one from the list.

Enter on the name of a `oneof` member lists all the members of the `oneof`: choosing
an absent one sets it with the default value, choosing the set message collapses or
expands it.

In a list, Up/Down, PgUp/PgDn and Home/End move the selection, a typed letter jumps to the
next item starting with it, Enter chooses the item and Esc closes the list.

F2 writes the result over the original file (through a temporary file) without making
a backup. Quitting with unsaved changes asks whether to save them.

## Hotkeys

F2 - Save file

F3 - Data view format: as in the proto file, decimal, hex (planned)

F4 - Change field sort order. Four variants available:

 * Proto - fields are shown in the order they are written in the proto file. This is the default mode.
 * Wire - fields are shown in the order they were read from the binary data file. In this mode only the data actually read from the file is shown (no default values).
 * Name - fields sorted by name.
 * Id - fields sorted by the numbers specified in the proto file.

 Shift+F4 switches the order backwards. The first char of the current order is shown in
 the right part of the top line, next to the position percentage.

F5/Enter - Expand/Collapse data, on a message or a collapsed field (Enter on a `oneof`
member lists the members of the `oneof` instead)

F6 - Comments visibility: hidden, inline, above the field (planned)

F10, Ctrl+C - Quit

Esc - Close the editor of the current field or a list, or quit if nothing is open

Ctrl+Up/Down - Navigate field of a message

Ctrl+Home/End - Go to the beginning or the end of the file

Del/Backspace - Delete data

Ins - Insert data

Inside a text field editor:

 * Shift+arrows, Shift+Home/End - select text
 * Alt+Shift+Up/Down - add a cursor on the line above/below
 * Ctrl+Z, Ctrl+Y (Ctrl+Shift+Z) - undo and redo the text edits (there is no
   document-wide undo yet)

## Limits

In the current version, the program may slow down when a single repeated field contains
more than 10 thousand items. Nested data may hold much more: the mega.pb example contains
1 million values, split between repeated fields of 100 items each.

The .proto parser supports a subset of the format: `message`, `enum`, `oneof`, `map`,
`reserved` and `option`, with `optional` and `repeated` fields; the options of fields and
enum values are skipped. Comments are not shown yet. proto2 files are read like proto3
ones: `required` fields are treated as `optional`, and default values set with
`[default = ...]` are not used. Groups, services and extensions are not supported.

## Examples

There are several example data files for testing the application in the 'resources' folder.
Type `protoedit resources/filename.pb` to open a file (or `cargo run --release -- resources/filename.pb`).

 * ints.pb - simple integer data example
 * str.pb - multiline string example
 * bytes.pb - a field with 1000 random bytes
 * enum.pb - enum fields: a short and a long list of values, a repeated enum, an absent
   field, a value missing from the enum
 * oneof.pb - `oneof` fields: in the root and in a nested message, with an enum and a message
   member, an empty `oneof` and an invalid one with two members set
 * mega.pb - 1 million random values in three-level structures
 * test_data_1.pb - simple nested data example

## Install

The app is published on [crates.io](https://crates.io/crates/protoedit).

Command `cargo install protoedit` will globally install the protoedit binary.
