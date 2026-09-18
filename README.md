# protoedit

Terminal-based [protobuf](https://protobuf.dev) data files editor.

Scalar, string and bytes fields are editable; enum fields can be viewed, but not edited yet.

Fields missing from the .proto file, for example when the file is written by a newer
version of the format, are shown as raw bytes and kept as they are, so a file saved
without changes is byte-identical to the original.

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
field commits the edit.

F2 writes the result over the original file (through a temporary file) without making
a backup, and quitting does not ask about unsaved changes.

## Hotkeys

F2 - Save file

F3 - Data view format: as in the proto file, decimal, hex (planned for the next version)

F4 - Change field sort order. Four variants available:

 * Proto - fields are shown in the order they are written in the proto file. This is the default mode.
 * Wire - fields are shown in the order they were read from the binary data file. In this mode only the data actually read from the file is shown (no default values).
 * Name - fields sorted by name.
 * Id - fields sorted by the numbers specified in the proto file.

 Shift+F4 switches the order backwards. The first char of the current order is shown in
 the right part of the top line, next to the position percentage.

F5/Enter - Expand/Collapse data, on a message or a collapsed field

F6 - Comments visibility: hidden, inline, above the field (planned for the next version)

F10 - Quit

Esc - Close the editor of the current field, or quit if no editor is open

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
`reserved` and `option`, with `optional` and `repeated` fields. Only `//` comments are
recognized, they are kept for the future versions, but not shown yet; `/* */` comments,
proto2 `required` fields, groups, services and extensions are not supported.

## Examples

There are several example data files for testing the application in the 'resources' folder.
Type `protoedit resources/filename.pb` to open a file (or `cargo run --release -- resources/filename.pb`).

 * ints.pb - simple integer data example
 * str.pb - multiline string example
 * bytes.pb - a field with 1000 random bytes
 * mega.pb - 1 million random values in three-level structures
 * test_data_1.pb - simple nested data example

## Install

The app is published on [crates.io](https://crates.io/crates/protoedit).

Command `cargo install protoedit` will globally install the protoedit binary.
