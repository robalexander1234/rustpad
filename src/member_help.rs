use crate::brace_matcher::char_to_byte_idx;
use crate::workspace::Document;
/// Option A + F1 Offline Docs:
/// Curated "Point & Shoot" Stdlib Method Browser + Local Type Heuristics + `rustup doc` launcher.
///
/// Features:
/// 1. Extracts the receiver expression before `.` at the cursor (e.g., `my_str.` or `my_vec.le`).
/// 2. Scans the current buffer and open documents for local variable type declarations:
///    - Explicit type annotations: `let s: &str`, `let mut v: Vec<f64>`, `fn foo(p: &Path)`
///    - Constructor/literal heuristics: `let s = "..."`, `let s = String::from(...)`, `let v = vec![...]`,
///      `let v = Vec::new()`, `let opt = Some(...)`, `let res = Ok(...)`, `let m = HashMap::new()`,
///      numeric literals (`0.0`, `42usize`), or local `struct` / `impl` blocks (`Point3D::new(...)`).
/// 3. Provides a fast, zero-lag categorized table of the most useful Rust stdlib methods
///    (`str / String`, `Vec / [T]`, `Option<T>`, `Result<T, E>`, `Iterator`, `Path / PathBuf`,
///    `f64 / Numbers`, `HashMap / HashSet`, plus dynamically scanned `Project Structs (impl)`).
/// 4. Clicking or pressing Enter shoots the method directly into the editor (placing the cursor
///    inside `(|)` when the method takes arguments).
/// 5. Pressing `F1` (or clicking "Open in Rust Docs (F1)") shells out to `rustup doc --std <query>`.
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeCategory {
    StrAndString,
    VecAndSlice,
    OptionType,
    ResultType,
    IteratorTrait,
    PathAndFile,
    NumericPrimitives,
    CollectionsMapSet,
    LocalProjectStructs,
}

impl TypeCategory {
    pub const ALL: &'static [TypeCategory] = &[
        TypeCategory::StrAndString,
        TypeCategory::VecAndSlice,
        TypeCategory::OptionType,
        TypeCategory::ResultType,
        TypeCategory::IteratorTrait,
        TypeCategory::PathAndFile,
        TypeCategory::NumericPrimitives,
        TypeCategory::CollectionsMapSet,
        TypeCategory::LocalProjectStructs,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            TypeCategory::StrAndString => "str / String",
            TypeCategory::VecAndSlice => "Vec<T> / [T]",
            TypeCategory::OptionType => "Option<T>",
            TypeCategory::ResultType => "Result<T, E>",
            TypeCategory::IteratorTrait => "Iterator",
            TypeCategory::PathAndFile => "Path / PathBuf",
            TypeCategory::NumericPrimitives => "f64 / usize / i32",
            TypeCategory::CollectionsMapSet => "HashMap / HashSet",
            TypeCategory::LocalProjectStructs => "Project impls",
        }
    }

    pub fn std_doc_keyword(&self) -> &'static str {
        match self {
            TypeCategory::StrAndString => "str",
            TypeCategory::VecAndSlice => "vec::Vec",
            TypeCategory::OptionType => "option::Option",
            TypeCategory::ResultType => "result::Result",
            TypeCategory::IteratorTrait => "iter::Iterator",
            TypeCategory::PathAndFile => "path::Path",
            TypeCategory::NumericPrimitives => "f64",
            TypeCategory::CollectionsMapSet => "collections::HashMap",
            TypeCategory::LocalProjectStructs => "std",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MethodEntry {
    pub name: String,
    pub insert_template: String, // Uses `|` to mark where the cursor lands
    pub signature: String,
    pub description: String,
    pub owner_type: String,
}

#[derive(Debug, Clone)]
pub struct ReceiverContext {
    pub receiver_ident: Option<String>,
    pub partial_method: String,
    pub replace_start_char: usize,
    pub replace_end_char: usize,
    pub inferred_category: Option<TypeCategory>,
    pub inferred_type_name: Option<String>,
}

/// Inspects the text immediately before `cursor_char_idx` to see if the user is typing
/// after a `.` (e.g., `my_var.` or `my_var.sp`) or on an identifier.
pub fn analyze_cursor_receiver(
    current_text: &str,
    cursor_char_idx: usize,
    all_docs: &[Document],
) -> ReceiverContext {
    let cursor_byte = char_to_byte_idx(current_text, cursor_char_idx);
    let bytes = current_text.as_bytes();

    // 1. Walk backward over any partial method identifier being typed right now
    let mut ident_start = cursor_byte;
    while ident_start > 0
        && (bytes[ident_start - 1].is_ascii_alphanumeric() || bytes[ident_start - 1] == b'_')
    {
        ident_start -= 1;
    }

    let partial_method = current_text[ident_start..cursor_byte].to_string();
    let replace_start_char = current_text[..ident_start].chars().count();

    // 2. Check if there is a `.` immediately preceding `ident_start`
    if ident_start > 0 && bytes[ident_start - 1] == b'.' {
        let dot_byte = ident_start - 1;
        // Skip optional `)` if chained like `foo().` or extract variable name before `.`
        let mut recv_end = dot_byte;
        while recv_end > 0 && bytes[recv_end - 1].is_ascii_whitespace() {
            recv_end -= 1;
        }

        let mut recv_start = recv_end;
        while recv_start > 0
            && (bytes[recv_start - 1].is_ascii_alphanumeric() || bytes[recv_start - 1] == b'_')
        {
            recv_start -= 1;
        }

        let receiver_ident = if recv_start < recv_end {
            Some(current_text[recv_start..recv_end].to_string())
        } else {
            None
        };

        let (inferred_category, inferred_type_name) = receiver_ident
            .as_deref()
            .and_then(|var| infer_variable_type(current_text, cursor_byte, var, all_docs))
            .map(|(cat, tname)| (Some(cat), Some(tname)))
            .unwrap_or((None, None));

        return ReceiverContext {
            receiver_ident,
            partial_method,
            replace_start_char,
            replace_end_char: cursor_char_idx,
            inferred_category,
            inferred_type_name,
        };
    }

    // 3. Otherwise, the cursor is on a standalone variable name (e.g. `origin` and user pressed Alt+M)
    if !partial_method.is_empty() {
        let (inferred_category, inferred_type_name) =
            infer_variable_type(current_text, cursor_byte, &partial_method, all_docs)
                .map(|(cat, tname)| (Some(cat), Some(tname)))
                .unwrap_or((None, None));

        return ReceiverContext {
            receiver_ident: Some(partial_method),
            partial_method: String::new(),
            replace_start_char: cursor_char_idx,
            replace_end_char: cursor_char_idx,
            inferred_category,
            inferred_type_name,
        };
    }

    ReceiverContext {
        receiver_ident: None,
        partial_method: String::new(),
        replace_start_char: cursor_char_idx,
        replace_end_char: cursor_char_idx,
        inferred_category: None,
        inferred_type_name: None,
    }
}

/// Heuristic local type scanner: looks backward from the cursor for `let [mut] <var> ...`
/// or function parameters `<var>: <Type>` and maps them to a `TypeCategory`.
fn infer_variable_type(
    text: &str,
    cursor_byte: usize,
    var_name: &str,
    all_docs: &[Document],
) -> Option<(TypeCategory, String)> {
    if var_name == "self" {
        // Find the enclosing `impl <StructName>` block before cursor_byte
        let prefix = &text[..cursor_byte.min(text.len())];
        for line in prefix.lines().rev() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("impl ") {
                let struct_name: String = rest
                    .trim()
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if !struct_name.is_empty() {
                    return Some((TypeCategory::LocalProjectStructs, struct_name));
                }
            }
        }
    }

    let local_structs = collect_project_struct_names(all_docs);
    let slice = &text[..cursor_byte.min(text.len())];

    // Scan lines in reverse from cursor so nearest shadowing declaration wins
    for line in slice.lines().rev() {
        let trimmed = line.trim();

        // 1. Check `let [mut] <var> : <Type>` or `let [mut] <var> = <Expr>`
        if let Some(after_let) = trimmed.strip_prefix("let ") {
            let after_mut = after_let
                .strip_prefix("mut ")
                .unwrap_or(after_let)
                .trim_start();

            if let Some(rest) = after_mut.strip_prefix(var_name) {
                let next_ch = rest.chars().next();
                if matches!(next_ch, Some(' ') | Some('\t') | Some(':') | Some('=')) {
                    if let Some((cat, name)) = classify_declaration_rhs(rest, &local_structs) {
                        return Some((cat, name));
                    }
                }
            }
        }

        // 2. Check function signature parameters `fn ... (..., var_name: &str, ...)`
        if trimmed.contains("fn ") && trimmed.contains(var_name) {
            let pattern = format!("{}:", var_name);
            if let Some(pos) = trimmed.find(&pattern) {
                let after_colon = &trimmed[pos + pattern.len()..];
                if let Some((cat, name)) = classify_declaration_rhs(after_colon, &local_structs) {
                    return Some((cat, name));
                }
            }
        }
    }

    None
}

fn classify_declaration_rhs(rhs: &str, local_structs: &[String]) -> Option<(TypeCategory, String)> {
    let s = rhs.trim();

    // Check local project structs first (e.g. `Point3D::new` or `: Point3D` or `: &Point3D`)
    for st in local_structs {
        if s.contains(st) {
            return Some((TypeCategory::LocalProjectStructs, st.clone()));
        }
    }

    // String / &str patterns
    if s.contains("&str")
        || s.contains("String")
        || s.contains("::from(\"")
        || s.contains(".to_string()")
        || s.contains("format!(")
        || contains_quoted_string_rhs(s)
    {
        return Some((TypeCategory::StrAndString, "String / &str".to_string()));
    }

    // Vec / Slice patterns
    if s.contains("Vec<")
        || s.contains("Vec::")
        || s.contains("vec![")
        || s.contains("&[")
        || s.contains(".collect::<Vec")
    {
        return Some((TypeCategory::VecAndSlice, "Vec<T> / &[T]".to_string()));
    }

    // Option patterns
    if s.contains("Option<") || s.contains("Some(") || s.ends_with("= None;") {
        return Some((TypeCategory::OptionType, "Option<T>".to_string()));
    }

    // Result patterns
    if s.contains("Result<") || s.contains("Ok(") || s.contains("Err(") || s.contains("fs::read") {
        return Some((TypeCategory::ResultType, "Result<T, E>".to_string()));
    }

    // Path / PathBuf patterns
    if s.contains("PathBuf") || s.contains("Path::new") || s.contains("&Path") {
        return Some((TypeCategory::PathAndFile, "Path / PathBuf".to_string()));
    }

    // HashMap / HashSet patterns
    if s.contains("HashMap") || s.contains("HashSet") || s.contains("BTreeMap") {
        return Some((
            TypeCategory::CollectionsMapSet,
            "HashMap / HashSet".to_string(),
        ));
    }

    // Iterator patterns
    if s.contains(".iter()") || s.contains(".chars()") || s.contains(".lines()") {
        return Some((TypeCategory::IteratorTrait, "Iterator".to_string()));
    }

    // Numeric primitives
    for num_ty in [
        "f64", "f32", "usize", "isize", "i32", "i64", "u32", "u64", "u8",
    ] {
        if s.contains(num_ty) {
            return Some((TypeCategory::NumericPrimitives, num_ty.to_string()));
        }
    }

    // Check bare numeric literal after `=`
    if let Some(eq_pos) = s.find('=') {
        let val = s[eq_pos + 1..].trim().trim_end_matches(';').trim();
        if !val.is_empty()
            && val
                .chars()
                .next()
                .map(|c| c.is_ascii_digit() || c == '-')
                .unwrap_or(false)
        {
            return Some((TypeCategory::NumericPrimitives, "f64 / integer".to_string()));
        }
    }

    None
}

fn contains_quoted_string_rhs(s: &str) -> bool {
    if let Some(eq_pos) = s.find('=') {
        let after = s[eq_pos + 1..].trim();
        after.starts_with('"') || after.starts_with("r#\"")
    } else {
        false
    }
}

pub fn collect_project_struct_names(all_docs: &[Document]) -> Vec<String> {
    let mut names = Vec::new();
    for doc in all_docs {
        for line in doc.content.lines() {
            let t = line.trim();
            let after = t
                .strip_prefix("pub struct ")
                .or_else(|| t.strip_prefix("struct "))
                .or_else(|| t.strip_prefix("impl "));
            if let Some(rest) = after {
                let ident: String = rest
                    .trim()
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if !ident.is_empty() && !names.contains(&ident) {
                    names.push(ident);
                }
            }
        }
    }
    names
}

/// Scans all `impl <Type>` blocks across open documents and extracts `fn <name>(...)` methods.
pub fn scan_local_impl_methods(all_docs: &[Document]) -> Vec<MethodEntry> {
    let mut entries = Vec::new();

    for doc in all_docs {
        let mut current_impl: Option<String> = None;
        let mut brace_depth: i32 = 0;

        for line in doc.content.lines() {
            let trimmed = line.trim();
            if current_impl.is_none() {
                if let Some(rest) = trimmed.strip_prefix("impl ") {
                    let header = rest.split('{').next().unwrap_or(rest).trim();
                    let type_name = header
                        .split_whitespace()
                        .last()
                        .unwrap_or(header)
                        .to_string();
                    if !type_name.is_empty() {
                        current_impl = Some(type_name);
                        brace_depth = line.chars().filter(|&c| c == '{').count() as i32
                            - line.chars().filter(|&c| c == '}').count() as i32;
                        continue;
                    }
                }
            } else {
                // Inside an `impl` block: look for `pub fn ` or `fn `
                let fn_rest = trimmed
                    .strip_prefix("pub fn ")
                    .or_else(|| trimmed.strip_prefix("fn "));
                if let Some(after_fn) = fn_rest {
                    let sig = after_fn.split('{').next().unwrap_or(after_fn).trim();
                    let fn_name: String = sig
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect();

                    if !fn_name.is_empty() {
                        let takes_extra_args = if let Some(lp) = sig.find('(') {
                            if let Some(rp) = sig.find(')') {
                                let params = sig[lp + 1..rp].trim();
                                let cleaned = params
                                    .trim_start_matches("&mut self")
                                    .trim_start_matches("&self")
                                    .trim_start_matches("self")
                                    .trim_start_matches(',')
                                    .trim();
                                !cleaned.is_empty()
                            } else {
                                false
                            }
                        } else {
                            false
                        };

                        let owner = current_impl.clone().unwrap_or_else(|| "Local".to_string());
                        let is_assoc = !sig.contains("self");
                        let insert_template = if takes_extra_args {
                            format!("{}(|)", fn_name)
                        } else {
                            format!("{}()", fn_name)
                        };

                        entries.push(MethodEntry {
                            name: fn_name,
                            insert_template,
                            signature: format!("fn {}", sig),
                            description: if is_assoc {
                                format!("Associated function on `{}` (in {})", owner, doc.title)
                            } else {
                                format!("Method on `{}` (in {})", owner, doc.title)
                            },
                            owner_type: owner,
                        });
                    }
                }

                brace_depth += line.chars().filter(|&c| c == '{').count() as i32;
                brace_depth -= line.chars().filter(|&c| c == '}').count() as i32;
                if brace_depth <= 0 {
                    current_impl = None;
                }
            }
        }
    }

    entries
}

/// Returns the curated list of methods for `category` (or dynamically scanned project methods).
pub fn methods_for_category(category: TypeCategory, all_docs: &[Document]) -> Vec<MethodEntry> {
    if category == TypeCategory::LocalProjectStructs {
        return scan_local_impl_methods(all_docs);
    }

    let raw: &[(&str, &str, &str, &str, &str)] = match category {
        TypeCategory::StrAndString => &[
            ("len", "len()", "fn len(&self) -> usize", "Returns the byte length of this string slice.", "str"),
            ("is_empty", "is_empty()", "fn is_empty(&self) -> bool", "Returns true if this string has a length of zero bytes.", "str"),
            ("trim", "trim()", "fn trim(&self) -> &str", "Returns a string slice with leading and trailing whitespace removed.", "str"),
            ("trim_start", "trim_start()", "fn trim_start(&self) -> &str", "Returns a string slice with leading whitespace removed.", "str"),
            ("trim_end", "trim_end()", "fn trim_end(&self) -> &str", "Returns a string slice with trailing whitespace removed.", "str"),
            ("contains", "contains(|)", "fn contains(&self, pat: Pattern) -> bool", "Returns true if the given pattern matches a sub-slice of this string.", "str"),
            ("starts_with", "starts_with(|)", "fn starts_with(&self, pat: Pattern) -> bool", "Returns true if the given pattern matches a prefix of this string.", "str"),
            ("ends_with", "ends_with(|)", "fn ends_with(&self, pat: Pattern) -> bool", "Returns true if the given pattern matches a suffix of this string.", "str"),
            ("find", "find(|)", "fn find(&self, pat: Pattern) -> Option<usize>", "Returns the byte index of the first character of this string that matches the pattern.", "str"),
            ("rfind", "rfind(|)", "fn rfind(&self, pat: Pattern) -> Option<usize>", "Returns the byte index of the first character of the last match of the pattern.", "str"),
            ("split", "split(|)", "fn split(&self, pat: Pattern) -> Split<'_, P>", "An iterator over substrings of this string slice, separated by characters matched by a pattern.", "str"),
            ("split_whitespace", "split_whitespace()", "fn split_whitespace(&self) -> SplitWhitespace<'_>", "Splits a string slice by whitespace.", "str"),
            ("lines", "lines()", "fn lines(&self) -> Lines<'_>", "An iterator over the lines of a string, as string slices.", "str"),
            ("chars", "chars()", "fn chars(&self) -> Chars<'_>", "Returns an iterator over the chars of a string slice.", "str"),
            ("char_indices", "char_indices()", "fn char_indices(&self) -> CharIndices<'_>", "Returns an iterator over the chars of a string slice and their byte positions.", "str"),
            ("bytes", "bytes()", "fn bytes(&self) -> Bytes<'_>", "An iterator over the bytes of a string slice.", "str"),
            ("replace", "replace(|, \"\")", "fn replace(&self, from: Pattern, to: &str) -> String", "Replaces all matches of a pattern with another string.", "str"),
            ("strip_prefix", "strip_prefix(|)", "fn strip_prefix(&self, prefix: Pattern) -> Option<&str>", "Returns a string slice with the prefix removed.", "str"),
            ("strip_suffix", "strip_suffix(|)", "fn strip_suffix(&self, suffix: Pattern) -> Option<&str>", "Returns a string slice with the suffix removed.", "str"),
            ("to_lowercase", "to_lowercase()", "fn to_lowercase(&self) -> String", "Returns the lowercase equivalent of this string slice as a new String.", "str"),
            ("to_uppercase", "to_uppercase()", "fn to_uppercase(&self) -> String", "Returns the uppercase equivalent of this string slice as a new String.", "str"),
            ("parse", "parse::<|>()", "fn parse<F: FromStr>(&self) -> Result<F, F::Err>", "Parses this string slice into another type (e.g. usize, f64).", "str"),
            ("to_string", "to_string()", "fn to_string(&self) -> String", "Converts the given value to an owned String.", "str"),
            ("as_str", "as_str()", "fn as_str(&self) -> &str", "Extracts a string slice containing the entire String.", "String"),
            ("push", "push(|)", "fn push(&mut self, ch: char)", "Appends the given char to the end of this String.", "String"),
            ("push_str", "push_str(|)", "fn push_str(&mut self, string: &str)", "Appends a given string slice onto the end of this String.", "String"),
            ("clear", "clear()", "fn clear(&mut self)", "Truncates this String, removing all contents.", "String"),
        ],
        TypeCategory::VecAndSlice => &[
            ("len", "len()", "fn len(&self) -> usize", "Returns the number of elements in the vector/slice.", "Vec / [T]"),
            ("is_empty", "is_empty()", "fn is_empty(&self) -> bool", "Returns true if the vector/slice contains no elements.", "Vec / [T]"),
            ("push", "push(|)", "fn push(&mut self, value: T)", "Appends an element to the back of a collection.", "Vec<T>"),
            ("pop", "pop()", "fn pop(&mut self) -> Option<T>", "Removes the last element from a vector and returns it, or None if empty.", "Vec<T>"),
            ("insert", "insert(|, val)", "fn insert(&mut self, index: usize, element: T)", "Inserts an element at position index within the vector.", "Vec<T>"),
            ("remove", "remove(|)", "fn remove(&mut self, index: usize) -> T", "Removes and returns the element at position index within the vector.", "Vec<T>"),
            ("clear", "clear()", "fn clear(&mut self)", "Clears the vector, removing all values.", "Vec<T>"),
            ("iter", "iter()", "fn iter(&self) -> Iter<'_, T>", "Returns an iterator over the slice.", "[T]"),
            ("iter_mut", "iter_mut()", "fn iter_mut(&mut self) -> IterMut<'_, T>", "Returns an iterator that allows modifying each value.", "[T]"),
            ("first", "first()", "fn first(&self) -> Option<&T>", "Returns the first element of the slice, or None if it is empty.", "[T]"),
            ("last", "last()", "fn last(&self) -> Option<&T>", "Returns the last element of the slice, or None if it is empty.", "[T]"),
            ("get", "get(|)", "fn get(&self, index: usize) -> Option<&T>", "Returns a reference to an element or subslice depending on the type of index.", "[T]"),
            ("contains", "contains(&|)", "fn contains(&self, x: &T) -> bool", "Returns true if the slice contains an element with the given value.", "[T]"),
            ("sort", "sort()", "fn sort(&mut self) where T: Ord", "Sorts the slice in-place.", "[T]"),
            ("sort_by", "sort_by(|a, b| a.cmp(b))", "fn sort_by<F>(&mut self, compare: F)", "Sorts the slice with a comparator function.", "[T]"),
            ("retain", "retain(|x| |)", "fn retain<F>(&mut self, f: F)", "Retains only the elements specified by the predicate.", "Vec<T>"),
            ("extend", "extend(|)", "fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I)", "Extends a collection with the contents of an iterator.", "Vec<T>"),
            ("join", "join(|)", "fn join(&self, sep: Separator) -> Output", "Flattens a slice of T into a single value, placing a separator between each.", "[T]"),
        ],
        TypeCategory::OptionType => &[
            ("is_some", "is_some()", "fn is_some(&self) -> bool", "Returns true if the option is a Some value.", "Option<T>"),
            ("is_none", "is_none()", "fn is_none(&self) -> bool", "Returns true if the option is a None value.", "Option<T>"),
            ("unwrap", "unwrap()", "fn unwrap(self) -> T", "Returns the contained Some value, panicking if the value is a None.", "Option<T>"),
            ("unwrap_or", "unwrap_or(|)", "fn unwrap_or(self, default: T) -> T", "Returns the contained Some value or a provided default.", "Option<T>"),
            ("unwrap_or_else", "unwrap_or_else(|| |)", "fn unwrap_or_else<F: FnOnce() -> T>(self, f: F) -> T", "Returns the contained Some value or computes it from a closure.", "Option<T>"),
            ("unwrap_or_default", "unwrap_or_default()", "fn unwrap_or_default(self) -> T where T: Default", "Returns the contained Some value or a default.", "Option<T>"),
            ("expect", "expect(\"|\")", "fn expect(self, msg: &str) -> T", "Returns the contained Some value, panicking with a custom message if None.", "Option<T>"),
            ("map", "map(|v| |)", "fn map<U, F: FnOnce(T) -> U>(self, f: F) -> Option<U>", "Maps an Option<T> to Option<U> by applying a function to a contained value.", "Option<T>"),
            ("and_then", "and_then(|v| |)", "fn and_then<U, F: FnOnce(T) -> Option<U>>(self, f: F) -> Option<U>", "Returns None if the option is None, otherwise calls f with the wrapped value.", "Option<T>"),
            ("or_else", "or_else(|| |)", "fn or_else<F: FnOnce() -> Option<T>>(self, f: F) -> Option<T>", "Returns the option if it contains a value, otherwise calls f.", "Option<T>"),
            ("filter", "filter(|v| |)", "fn filter<P: FnOnce(&T) -> bool>(self, predicate: P) -> Option<T>", "Returns None if the option is None, otherwise calls predicate with the wrapped value.", "Option<T>"),
            ("as_ref", "as_ref()", "fn as_ref(&self) -> Option<&T>", "Converts from &Option<T> to Option<&T>.", "Option<T>"),
            ("as_deref", "as_deref()", "fn as_deref(&self) -> Option<&T::Target>", "Converts from Option<T> (or &Option<T>) to Option<&T::Target>.", "Option<T>"),
            ("take", "take()", "fn take(&mut self) -> Option<T>", "Takes the value out of the option, leaving a None in its place.", "Option<T>"),
            ("ok_or", "ok_or(|)", "fn ok_or<E>(self, err: E) -> Result<T, E>", "Transforms the Option<T> into a Result<T, E>, mapping Some(v) to Ok(v) and None to Err(err).", "Option<T>"),
        ],
        TypeCategory::ResultType => &[
            ("is_ok", "is_ok()", "fn is_ok(&self) -> bool", "Returns true if the result is Ok.", "Result<T, E>"),
            ("is_err", "is_err()", "fn is_err(&self) -> bool", "Returns true if the result is Err.", "Result<T, E>"),
            ("ok", "ok()", "fn ok(self) -> Option<T>", "Converts from Result<T, E> to Option<T>, discarding the error.", "Result<T, E>"),
            ("err", "err()", "fn err(self) -> Option<E>", "Converts from Result<T, E> to Option<E>, discarding the success value.", "Result<T, E>"),
            ("unwrap", "unwrap()", "fn unwrap(self) -> T", "Returns the contained Ok value, panicking if Err.", "Result<T, E>"),
            ("expect", "expect(\"|\")", "fn expect(self, msg: &str) -> T", "Returns the contained Ok value, panicking with msg if Err.", "Result<T, E>"),
            ("unwrap_or", "unwrap_or(|)", "fn unwrap_or(self, default: T) -> T", "Returns the contained Ok value or a provided default.", "Result<T, E>"),
            ("unwrap_or_else", "unwrap_or_else(|e| |)", "fn unwrap_or_else<F: FnOnce(E) -> T>(self, op: F) -> T", "Returns the contained Ok value or computes it from a closure.", "Result<T, E>"),
            ("map", "map(|v| |)", "fn map<U, F: FnOnce(T) -> U>(self, op: F) -> Result<U, E>", "Maps a Result<T, E> to Result<U, E> by applying a function to a contained Ok value.", "Result<T, E>"),
            ("map_err", "map_err(|e| |)", "fn map_err<F, O: FnOnce(E) -> F>(self, op: O) -> Result<T, F>", "Maps a Result<T, E> to Result<T, F> by applying a function to a contained Err value.", "Result<T, E>"),
            ("and_then", "and_then(|v| |)", "fn and_then<U, F: FnOnce(T) -> Result<U, E>>(self, op: F) -> Result<U, E>", "Calls op if the result is Ok, otherwise returns the Err value.", "Result<T, E>"),
        ],
        TypeCategory::IteratorTrait => &[
            ("map", "map(|x| |)", "fn map<B, F>(self, f: F) -> Map<Self, F>", "Takes a closure and creates an iterator which calls that closure on each element.", "Iterator"),
            ("filter", "filter(|x| |)", "fn filter<P>(self, predicate: P) -> Filter<Self, P>", "Creates an iterator which uses a closure to determine if an element should be yielded.", "Iterator"),
            ("filter_map", "filter_map(|x| |)", "fn filter_map<B, F>(self, f: F) -> FilterMap<Self, F>", "Creates an iterator that both filters and maps.", "Iterator"),
            ("enumerate", "enumerate()", "fn enumerate(self) -> Enumerate<Self>", "Creates an iterator which gives the current iteration count as well as the next value.", "Iterator"),
            ("collect", "collect::<Vec<_>>()", "fn collect<B: FromIterator<Self::Item>>(self) -> B", "Transforms an iterator into a collection (such as Vec, String, HashMap).", "Iterator"),
            ("find", "find(|x| |)", "fn find<P>(&mut self, predicate: P) -> Option<Self::Item>", "Searches for an element of an iterator that satisfies a predicate.", "Iterator"),
            ("position", "position(|x| |)", "fn position<P>(&mut self, predicate: P) -> Option<usize>", "Searches for an element in an iterator, returning its index.", "Iterator"),
            ("any", "any(|x| |)", "fn any<F>(&mut self, f: F) -> bool", "Tests if any element of the iterator matches a predicate.", "Iterator"),
            ("all", "all(|x| |)", "fn all<F>(&mut self, f: F) -> bool", "Tests if every element of the iterator matches a predicate.", "Iterator"),
            ("count", "count()", "fn count(self) -> usize", "Consumes the iterator, counting the number of iterations and returning it.", "Iterator"),
            ("sum", "sum()", "fn sum<S>(self) -> S", "Sums the elements of an iterator.", "Iterator"),
            ("fold", "fold(0, |acc, x| |)", "fn fold<B, F>(self, init: B, f: F) -> B", "Folds every element into an accumulator by applying an operation, returning the final result.", "Iterator"),
            ("take", "take(|)", "fn take(self, n: usize) -> Take<Self>", "Creates an iterator that yields the first n elements, or fewer if the underlying iterator ends sooner.", "Iterator"),
            ("skip", "skip(|)", "fn skip(self, n: usize) -> Skip<Self>", "Creates an iterator that skips the first n elements.", "Iterator"),
            ("rev", "rev()", "fn rev(self) -> Rev<Self>", "Reverses an iterator's direction.", "Iterator"),
            ("zip", "zip(|)", "fn zip<U>(self, other: U) -> Zip<Self, U::IntoIter>", "Zips up two iterators into a single iterator of pairs.", "Iterator"),
            ("cloned", "cloned()", "fn cloned<'a, T>(self) -> Cloned<Self>", "Creates an iterator which clones all of its elements.", "Iterator"),
        ],
        TypeCategory::PathAndFile => &[
            ("exists", "exists()", "fn exists(&self) -> bool", "Returns true if the path points at an existing entity.", "Path"),
            ("is_file", "is_file()", "fn is_file(&self) -> bool", "Returns true if the path exists on disk and is pointing at a regular file.", "Path"),
            ("is_dir", "is_dir()", "fn is_dir(&self) -> bool", "Returns true if the path exists on disk and is pointing at a directory.", "Path"),
            ("join", "join(|)", "fn join<P: AsRef<Path>>(&self, path: P) -> PathBuf", "Creates an owned PathBuf with path adjoined to self.", "Path"),
            ("parent", "parent()", "fn parent(&self) -> Option<&Path>", "Returns the Path without its final component, if there is one.", "Path"),
            ("file_name", "file_name()", "fn file_name(&self) -> Option<&OsStr>", "Returns the final component of the Path, if there is one.", "Path"),
            ("extension", "extension()", "fn extension(&self) -> Option<&OsStr>", "Extracts the extension (without the leading dot) of self.file_name, if possible.", "Path"),
            ("display", "display()", "fn display(&self) -> Display<'_>", "Returns an object that implements Display for safely printing paths.", "Path"),
            ("to_str", "to_str()", "fn to_str(&self) -> Option<&str>", "Yields a &str slice if the Path is valid unicode.", "Path"),
            ("to_path_buf", "to_path_buf()", "fn to_path_buf(&self) -> PathBuf", "Converts a Path to an owned PathBuf.", "Path"),
            ("strip_prefix", "strip_prefix(|)", "fn strip_prefix<P>(&self, base: P) -> Result<&Path, StripPrefixError>", "Returns a relative path from base to self.", "Path"),
            ("with_extension", "with_extension(|)", "fn with_extension<S: AsRef<OsStr>>(&self, extension: S) -> PathBuf", "Creates an owned PathBuf like self but with the given extension.", "Path"),
        ],
        TypeCategory::NumericPrimitives => &[
            ("abs", "abs()", "fn abs(self) -> Self", "Computes the absolute value of self.", "f64 / i32"),
            ("sqrt", "sqrt()", "fn sqrt(self) -> f64", "Returns the square root of a number.", "f64 / f32"),
            ("sin", "sin()", "fn sin(self) -> f64", "Computes the sine of a number (in radians).", "f64 / f32"),
            ("cos", "cos()", "fn cos(self) -> f64", "Computes the cosine of a number (in radians).", "f64 / f32"),
            ("powi", "powi(|)", "fn powi(self, n: i32) -> f64", "Raises a number to an integer power.", "f64 / f32"),
            ("powf", "powf(|)", "fn powf(self, n: f64) -> f64", "Raises a number to a floating point power.", "f64 / f32"),
            ("floor", "floor()", "fn floor(self) -> f64", "Returns the largest integer less than or equal to self.", "f64 / f32"),
            ("ceil", "ceil()", "fn ceil(self) -> f64", "Returns the smallest integer greater than or equal to self.", "f64 / f32"),
            ("round", "round()", "fn round(self) -> f64", "Returns the nearest integer to self.", "f64 / f32"),
            ("min", "min(|)", "fn min(self, other: Self) -> Self", "Returns the minimum of the two numbers.", "Number"),
            ("max", "max(|)", "fn max(self, other: Self) -> Self", "Returns the maximum of the two numbers.", "Number"),
            ("clamp", "clamp(|, max)", "fn clamp(self, min: Self, max: Self) -> Self", "Restrict a value to a certain interval.", "Number"),
            ("saturating_sub", "saturating_sub(|)", "fn saturating_sub(self, rhs: Self) -> Self", "Saturating integer subtraction (clamps at 0 for unsigned).", "usize / integer"),
            (" saturating_add", "saturating_add(|)", "fn saturating_add(self, rhs: Self) -> Self", "Saturating integer addition.", "usize / integer"),
        ],
        TypeCategory::CollectionsMapSet => &[
            ("insert", "insert(|, val)", "fn insert(&mut self, k: K, v: V) -> Option<V>", "Inserts a key-value pair into the map (or value into the set).", "HashMap / HashSet"),
            ("get", "get(&|)", "fn get<Q>(&self, k: &Q) -> Option<&V>", "Returns a reference to the value corresponding to the key.", "HashMap"),
            ("get_mut", "get_mut(&|)", "fn get_mut<Q>(&mut self, k: &Q) -> Option<&mut V>", "Returns a mutable reference to the value corresponding to the key.", "HashMap"),
            ("contains_key", "contains_key(&|)", "fn contains_key<Q>(&self, k: &Q) -> bool", "Returns true if the map contains a value for the specified key.", "HashMap"),
            ("contains", "contains(&|)", "fn contains<Q>(&self, value: &Q) -> bool", "Returns true if the set contains a value.", "HashSet"),
            ("remove", "remove(&|)", "fn remove<Q>(&mut self, k: &Q) -> Option<V>", "Removes a key from the map, returning the value at the key if the key was previously in the map.", "HashMap"),
            ("entry", "entry(|).or_insert(default)", "fn entry(&mut self, key: K) -> Entry<'_, K, V>", "Gets the given key's corresponding entry in the map for in-place manipulation.", "HashMap"),
            ("keys", "keys()", "fn keys(&self) -> Keys<'_, K, V>", "An iterator visiting all keys in arbitrary order.", "HashMap"),
            ("values", "values()", "fn values(&self) -> Values<'_, K, V>", "An iterator visiting all values in arbitrary order.", "HashMap"),
            ("len", "len()", "fn len(&self) -> usize", "Returns the number of elements in the map/set.", "HashMap / HashSet"),
        ],
        TypeCategory::LocalProjectStructs => &[],
    };

    raw.iter()
        .map(|(name, tmpl, sig, desc, owner)| MethodEntry {
            name: (*name).to_string(),
            insert_template: (*tmpl).to_string(),
            signature: (*sig).to_string(),
            description: (*desc).to_string(),
            owner_type: (*owner).to_string(),
        })
        .collect()
}

/// Inserts the chosen `MethodEntry` into `text`, replacing `replace_start_char..replace_end_char`
/// (and inserting a leading `.` if the cursor wasn't already preceded by a `.`).
/// Returns the new cursor position in `char` coordinates.
pub fn apply_method_insertion(
    text: &mut String,
    replace_start_char: usize,
    replace_end_char: usize,
    entry: &MethodEntry,
) -> usize {
    let start_byte = char_to_byte_idx(text, replace_start_char);
    let end_byte = char_to_byte_idx(text, replace_end_char);

    let has_preceding_dot = start_byte > 0 && text.as_bytes()[start_byte - 1] == b'.';
    let prefix_dot = if has_preceding_dot { "" } else { "." };

    let full_template = format!("{}{}", prefix_dot, entry.insert_template);
    let pipe_byte_offset = full_template.find('|').unwrap_or(full_template.len());
    let clean_text = full_template.replacen('|', "", 1);

    text.replace_range(start_byte..end_byte, &clean_text);

    let new_cursor_byte = start_byte + pipe_byte_offset;
    text[..new_cursor_byte].chars().count()
}

/// Extracts the identifier word currently under `cursor_char_idx` (used for `F1` doc lookup).
pub fn word_at_cursor(text: &str, cursor_char_idx: usize) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    let cursor_byte = char_to_byte_idx(text, cursor_char_idx).min(text.len());
    let bytes = text.as_bytes();

    let mut start = cursor_byte;
    while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
        start -= 1;
    }

    let mut end = cursor_byte;
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }

    if start < end {
        Some(text[start..end].to_string())
    } else {
        None
    }
}

/// Launches `rustup doc --std <symbol>` (or opens the offline Rust standard library docs)
/// on a detached background thread so the editor UI never blocks.
pub fn open_offline_rust_docs(query: &str) -> String {
    let cleaned = query.trim().to_string();
    std::thread::spawn(move || {
        let mut cmd = Command::new("rustup");
        cmd.arg("doc");
        cmd.arg("--std");
        if !cleaned.is_empty() {
            cmd.arg(&cleaned);
        }
        // If `rustup doc --std <symbol>` returns non-zero (e.g. method name not a top-level item),
        // fall back to opening `rustup doc --std` so the user lands right on the offline search page.
        if let Ok(status) = cmd.status() {
            if !status.success() {
                let _ = Command::new("rustup").arg("doc").arg("--std").spawn();
            }
        } else {
            let _ = Command::new("rustup").arg("doc").arg("--std").spawn();
        }
    });

    if query.trim().is_empty() {
        "Opened offline Rust standard library docs (`rustup doc --std`).".to_string()
    } else {
        format!(
            "Launched offline Rust docs (`rustup doc --std {}`).",
            query.trim()
        )
    }
}
