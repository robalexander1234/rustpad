# RustPad

**RustPad** is a fast, lightweight, PSPad-inspired programmer's editor written in pure Rust, purpose-built for Rust development[cite: 30, 40]. 

Modern IDEs often bundle full web browsers or require gigabytes of RAM just to edit a few source files. RustPad takes a pragmatic, first-principles approach: it compiles to a single native executable, launches instantaneously, and provides essential code-editing and `cargo` build-tool integration with minimal external dependencies.

---

## Key Features

* **Multi-File Tabbed Workspace:** Open, edit, and switch between multiple source files simultaneously (`Ctrl+Tab`), with per-file undo/redo history (`Ctrl+Z` / `Ctrl+Y`), dirty-state indicators (`*`), and one-key **Save All** (`Ctrl+Shift+S`).
* **Zero-Dependency Rust Syntax Highlighter:** Built-in single-pass lexical scanner that colors Rust keywords, standard library and `UpperCamelCase` types, string and character literals, lifetimes (`'a`), macros (`println!`), attributes (`#[derive(...)]`), numeric literals, and nested block comments (`/* ... */`)[cite: 2, 47]. Switchable on the fly between a **Dark Syntax Theme** and a **Classic PSPad Light Theme**[cite: 2, 40, 42].
* **Context-Aware Brace & Delimiter Matching:** Live highlighting of matching `{...}`, `(...)`, and `[...]` pairs whenever the caret rests on or adjacent to a delimiter, complete with exact `Ln, Col` coordinate readouts in the status bar[cite: 2, 42, 48]. Automatically ignores delimiters inside string literals, character literals, and comments[cite: 2, 48].
* **PSPad-Style Clip / Template Expansion (`Ctrl+J` / `Ctrl+Space`):** Type a short trigger word (such as `fn`, `pfn`, `main`, `for`, `if`, `ifel`, `match`, `struct`, `enum`, `impl`, `test`, `pr`, or `epr`) and press `Ctrl+J` to expand it in place with automatic indentation and caret placement[cite: 2, 40, 46]. Clips can also be inserted from the **Misc** menu or the toggleable **Clips Sidebar**.
* **Async Cargo Build Runner & Clickable Diagnostics:** Run `cargo check` (`F9`), `cargo build` (`Ctrl+F9`), `cargo clippy`, `cargo test` (`Shift+F9`), or `cargo run` (`F5`) on a background thread without freezing the UI[cite: 30, 40, 43]. Compiler errors and warnings are parsed from `rustc`'s JSON output into a split bottom dock: click any diagnostic on the left to open the file, jump to the exact line and column, and highlight the offending span while inspecting the full borrow-checker explanation on the right.
* **Single-File & Multi-File Search ("Find in Files"):** 
  * **In-File Find & Replace (`Ctrl+F` / `Ctrl+H`):** Case-sensitive or case-insensitive search with forward/backward navigation (`F3` / `Shift+F3`) and visible match selection[cite: 30, 40, 42, 44].
  * **Find in Files (`Ctrl+Shift+F`):** Recursively search across your project directory tree (automatically skipping `target/` and `.git/`) or across all open tabs by file extension (`rs, toml, md`)[cite: 30, 42, 44]. Click any match in the bottom **Search Results** tab to jump straight to that file and line[cite: 42].
* **Block Line Comment Toggling (`Ctrl+/` or `Ctrl+Shift+A`):** Comment or uncomment single lines or multi-line selections with `// `[cite: 2, 40, 45].

---

## Prerequisites & Dependencies

To build and run RustPad, you only need a standard Rust toolchain (`rustc` and `cargo`, edition 2021 or newer)[cite: 2, 41].

RustPad keeps its dependency tree intentionally small[cite: 41]:
* [`eframe` / `egui`](https://github.com/emilk/egui) (`0.29`) — Immediate-mode native GUI windowing and text rendering[cite: 41, 42].
* [`rfd`](https://github.com/PolyMeilex/rfd) (`0.15`) — Native OS file and folder selection dialogs[cite: 41, 42].
* [`serde`](https://serde.rs/) & [`serde_json`](https://github.com/serde-rs/json) (`1.0`) — Parsing structured `--message-format=json` compiler diagnostics from Cargo[cite: 41, 43].

---

## Building and Running

Clone the repository and build the optimized release binary using Cargo:

```bash
git clone [https://github.com/robalexander1234/rustpad.git](https://github.com/robalexander1234/rustpad.git)
cd rustpad
cargo run --release