# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

protoedit is a terminal-based (crossterm TUI) editor/viewer for binary protobuf data files. It reads a `.pb` data file plus a `.proto` schema and presents the decoded message tree for navigation and editing. Currently it functions mostly as a viewer — not all field types are editable yet.

## Commands

```bash
cargo build                 # debug build
cargo build --release       # release build (use this for real files; debug is slow)
cargo test                  # run all tests
cargo test <name>           # run a single test, e.g. `cargo test app_test_1`
cargo test -- --nocapture   # show stdout from tests
cargo run -- resources/ints.pb          # run against an example (auto-derives ints.proto)
cargo run -- "data.pb;schema.proto;Root"  # explicit schema + root message
```

CI (`.github/workflows/rust.yml`) runs only `cargo build` and `cargo test` — there is no configured lint/format step.

### CLI argument format

A single positional argument, semicolon-separated: `data.pb;format.proto;message_name`.
- Only `data.pb` is required. If the `.proto` is omitted it defaults to the data file's name with a `.proto` extension.
- If `message_name` (the root message) is omitted, it is auto-detected via `ProtoData::auto_detect_root_message()` (the one message type not used as a field of any other).
- `-I`/`--proto_path <dir>` adds directories for resolving `import`ed proto files (must be absolute paths).

`resources/` holds example `.pb`/`.proto` pairs (`ints`, `str`, `bytes`, `mega`, `test_data_1`) for manual testing.

## Architecture

Two parallel data models flow through the program and must be kept consistent with each other:

- **Schema** — the parsed `.proto`, owned by `ProtoData` (`proto.rs`) and the per-type `FieldProto` implementations (`typedefs.rs`). Immutable and `Rc`-shared (`MessageProtoPtr`, `FieldProtoPtr`, `EnumProtoPtr`) after construction.
- **Data** — the decoded binary, a tree of `MessageData` → `FieldData` → `FieldValue` (`wire.rs`). Mutable; this is what the user edits and what gets written back.

### Module map

| File | Role |
|------|------|
| `main.rs` | `App`: terminal setup, crossterm event loop, key/mouse dispatch, screen rendering. Also CLI parsing in `main()`. |
| `proto.rs` | `.proto` parsing into `ProtoData`/`MessageProto`/`EnumProto`. Root-message auto-detection, map-type synthesis, type linking. |
| `typedefs.rs` | Binary I/O primitives (`PbReader`/`PbReaderTrait`) and the `FieldProto` trait with one struct per scalar protobuf type. |
| `wire.rs` | Runtime data model (`MessageData`, `FieldData`, `ScalarValue`), binary read/write, and `FieldPath` navigation. |
| `view.rs` | Presentation: `Layouts`, the `ViewLayout` trait + implementations, `UserCommand`, `CommandResult`, `FieldOrder`, `LayoutConfig`, `TextStyle`. |
| `trz.rs` | Mutation records (`Change`, `ChangeType`, `History`) used for editing and undo. |
| `text_edit.rs` | In-place text/string editing helpers (`TextEditor`, `TextLines`). |
| `pb.pest` | PEG grammar (pest) for the `.proto` subset that is supported. |

### Critical invariants

- **`ProtoData::finalize()` is mandatory** before decoding data. It synthesizes hidden message types for `map<>` fields, sorts messages/enums by name (later lookups use `binary_search_by`, so order matters), and runs `link_user_types`, which resolves each `EnumOrMessageFieldDefinition` to an enum or a message via `OnceCell`. Before linking, a user-typed field does not even know whether it is a message or an enum, and `read()`/`is_message()` will misbehave.
- **Byte-exact round-trip.** Fields absent from the schema are preserved as `ScalarValue::UNKNOWN` (tag + raw bytes), so an unmodified file written back is byte-identical to the input. Many tests assert `output == binary_input` — preserve this when touching read/write paths.
- **`FieldProto` impls and `ScalarValue` variants are paired.** Each type struct's `read`/`write`/`default` expects one specific `ScalarValue` variant and `unreachable!()`s otherwise. To add a protobuf type you add a struct in `typedefs.rs`, register it in `CommonFieldProto::new_field`, and add the matching `ScalarValue` variant in `wire.rs`.
- **The `.proto` parser is a deliberate subset**, not full protobuf. Only `//` single-line comments (captured for display), `message`/`enum`/`oneof`/`map`/`reserved`/`option`; `syntax`/`package`/`import` lines are skipped. Constructs outside the grammar (block comments, etc.) will fail to parse.

### Editing & command flow

1. A key event in `App::on_key` becomes a `UserCommand` (`view.rs`).
2. `App::run_command` → `Layouts::run_command`, which usually delegates to the selected layout's `ViewLayout::on_command`.
3. The result is a `CommandResult`. `CommandResult::ChangeData(Change)` is applied via `MessageData::apply(&mut Change)`.
4. `MessageData::apply` performs the edit **and rewrites the `Change` into its inverse** (e.g. `Insert` becomes `Delete`), which is the basis for undo/redo (`trz.rs`).
5. Navigation into the data tree uses `FieldPath` — a `Vec<FieldPos { id, index }>` where `index > 0` selects among repeated fields.
6. Saving (F2) calls `Layouts::save_document`, which writes to a `.tmp` file and atomically renames it over the original.

### Lazy layout model (why scrolling stays fast)

`Layouts.items` is a flat `Vec<LayoutParams>`, one entry per visible field or repeated-field group. Nested message contents are **not** built up front: child layouts start as `LayoutParams::new_empty` (no `Box<dyn ViewLayout>`) and are materialized on demand by `Layouts::ensure_loaded` as the viewport scrolls. This bounded, on-demand construction is what keeps large files usable (the README notes slowdowns past ~10k items). `ViewLayout` implementations: `ScalarLayout`, `StringLayout`, `BytesLayout`, `MessageLayout`, `TableLayout`, `CollapsedLayout`. `FieldOrder` (Proto/Wire/ByName/ById) controls field ordering and is cycled with F4.

### Tests

Tests are inline `#[cfg(test)]` modules, concentrated in `main.rs` (app/rendering) and `wire.rs` (decode/encode). They build data from raw byte arrays plus an inline proto string, then either compare `MessageData::to_string()` or use `App::for_tests` (a headless `App` with no real terminal) and assert on `App::to_strings()`, which renders the screen to a `Vec<String>`. These golden-string assertions are width-sensitive and assume `MARGIN_LEFT == MARGIN_RIGHT == 1` (guarded by the `match_testing_requirements` test).

Note: `main.rs` and `view.rs` begin with `#![allow(warnings)]` and contain substantial commented-out/WIP code — the project is under active development.
