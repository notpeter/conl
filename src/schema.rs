//! Schema validation for CONL documents.
//!
//! This module provides a mechanism to validate the structure of a CONL document
//! against a schema definition. Schemas are themselves CONL documents with a
//! specific structure.
//!
//! # Example
//!
//! ```
//! use conl::schema::Schema;
//!
//! let schema_input = br#"
//! root = <server>
//! definitions
//!   server
//!     required keys
//!       type = server
//!     keys
//!       host = .*
//!       port = \d+
//! "#;
//!
//! let schema = Schema::parse(schema_input).unwrap();
//! let input = br#"
//! type = server
//! host = localhost
//! port = 8080
//! "#;
//!
//! let result = schema.validate(input);
//! assert!(result.is_valid());
//! ```

use crate::{parse, SyntaxError, Token};
use regex::Regex;
use std::borrow::Cow;
use std::collections::HashMap;

/// A Schema allows you to validate a CONL document against a set of rules.
#[derive(Debug)]
pub struct Schema {
    root: Matcher,
    definitions: HashMap<String, Definition>,
}

/// A Definition describes how to validate a value in a CONL document.
#[derive(Debug, Default, Clone)]
pub struct Definition {
    pub name: String,
    pub docs: String,

    pub scalar: Option<Matcher>,
    pub any_of: Vec<Matcher>,

    pub keys: Vec<(Matcher, Matcher)>,
    pub required_keys: Vec<(Matcher, Matcher)>,

    pub items: Option<Matcher>,
    pub required_items: Vec<Matcher>,
}

/// A Matcher describes what values are valid in a particular context.
#[derive(Debug, Clone)]
pub struct Matcher {
    pub pattern: Option<Regex>,
    pub reference: Option<String>,
    pub docs: String,
    pub raw: String,
}

impl Default for Matcher {
    fn default() -> Self {
        Self {
            pattern: None,
            reference: None,
            docs: String::new(),
            raw: String::new(),
        }
    }
}

impl Matcher {
    fn from_str(s: &str) -> Result<Self, SchemaError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(SchemaError::new(0, "empty matcher"));
        }

        if s.starts_with('<') {
            if !s.ends_with('>') {
                return Err(SchemaError::new(0, "missing closing >"));
            }
            let reference = &s[1..s.len() - 1];
            Ok(Matcher {
                pattern: None,
                reference: Some(reference.to_string()),
                docs: String::new(),
                raw: s.to_string(),
            })
        } else {
            let pattern_str = format!("(?s)^{}$", s);
            let pattern = Regex::new(&pattern_str).map_err(|e| {
                SchemaError::new(0, format!("invalid regex pattern '{}': {}", s, e))
            })?;
            Ok(Matcher {
                pattern: Some(pattern),
                reference: None,
                docs: String::new(),
                raw: s.to_string(),
            })
        }
    }

    fn resolve(
        &self,
        schema: &Schema,
        seen: &mut Vec<String>,
        resolved: &mut std::collections::HashSet<String>,
    ) -> Result<(), SchemaError> {
        if self.pattern.is_some() {
            return Ok(());
        }

        if let Some(ref reference) = self.reference {
            if !schema.definitions.contains_key(reference) {
                return Err(SchemaError::new(
                    0,
                    format!("<{}> is not defined", reference),
                ));
            }
            // Currently being resolved in this chain - circular reference
            // Check seen FIRST before resolved to detect cycles in current path
            if seen.contains(reference) {
                return Err(SchemaError::new(
                    0,
                    format!("<{}> is defined in terms of itself", reference),
                ));
            }
            // Already started resolving, skip (allows structural recursion)
            if resolved.contains(reference) {
                return Ok(());
            }
            // Mark as being resolved BEFORE recursing to prevent infinite recursion
            resolved.insert(reference.clone());
            seen.push(reference.clone());
            let def = schema.definitions.get(reference).unwrap();
            def.resolve(schema, seen, resolved)?;
            seen.pop();
        }
        Ok(())
    }

    fn get_definition<'a>(&self, schema: &'a Schema) -> Option<&'a Definition> {
        self.reference
            .as_ref()
            .and_then(|r| schema.definitions.get(r))
    }

    fn validate(&self, val: &ConlValue, schema: &Schema, pos: ResultPos) -> InternalResult {
        if let Some(def) = self.get_definition(schema) {
            return def.validate(val, schema, pos, self);
        }

        // Pattern matcher
        if let Some(ref pattern) = self.pattern {
            if let Some(scalar) = &val.scalar {
                if pattern.is_match(&scalar.content) {
                    return InternalResult::new(pos, Attempt::value(val, true, Some(self)));
                }
            }
            return InternalResult::new(pos, Attempt::value(val, false, Some(self)));
        }

        InternalResult::new(pos, Attempt::value(val, false, Some(self)))
    }

    fn suggested_values(&self, schema: &Schema) -> Vec<Suggestion> {
        if let Some(def) = self.get_definition(schema) {
            let mut suggestions = Vec::new();
            if let Some(ref scalar) = def.scalar {
                suggestions.extend(scalar.suggested_values(schema));
            }
            for any_of in &def.any_of {
                suggestions.extend(any_of.suggested_values(schema));
            }
            for s in &mut suggestions {
                if s.docs.is_empty() {
                    s.docs = self.docs.clone();
                }
            }
            return suggestions;
        }

        let mut suggestions = Vec::new();
        for s in suggestions_from_pattern(&self.raw, false) {
            suggestions.push(Suggestion {
                value: s,
                docs: self.docs.clone(),
            });
        }
        suggestions
    }
}

impl Definition {
    fn resolve(
        &self,
        schema: &Schema,
        seen: &mut Vec<String>,
        resolved: &mut std::collections::HashSet<String>,
    ) -> Result<(), SchemaError> {
        let count = [
            self.scalar.is_some(),
            !self.any_of.is_empty(),
            !self.keys.is_empty() || !self.required_keys.is_empty(),
            self.items.is_some() || !self.required_items.is_empty(),
        ]
        .iter()
        .filter(|&&b| b)
        .count();

        if count > 1 {
            return Err(SchemaError::new(
                0,
                format!(
                    "invalid definition {}: cannot mix scalar, any of, (required) keys, and (required) items",
                    self.name
                ),
            ));
        }

        // Scalars and any_of use the same seen list (to detect direct circular refs)
        if let Some(ref scalar) = self.scalar {
            scalar.resolve(schema, seen, resolved)?;
        }
        for choice in &self.any_of {
            choice.resolve(schema, seen, resolved)?;
        }
        // Keys and items use a fresh seen list (to allow structural recursion)
        for (key, val) in &self.keys {
            key.resolve(schema, &mut vec![], resolved)?;
            val.resolve(schema, &mut vec![], resolved)?;
        }
        for (key, val) in &self.required_keys {
            key.resolve(schema, &mut vec![], resolved)?;
            val.resolve(schema, &mut vec![], resolved)?;
        }
        if let Some(ref items) = self.items {
            items.resolve(schema, &mut vec![], resolved)?;
        }
        for item in &self.required_items {
            item.resolve(schema, &mut vec![], resolved)?;
        }
        Ok(())
    }

    fn validate(
        &self,
        val: &ConlValue,
        schema: &Schema,
        pos: ResultPos,
        matcher: &Matcher,
    ) -> InternalResult {
        if let Some(ref scalar) = self.scalar {
            return scalar.validate(val, schema, pos);
        }

        if !self.any_of.is_empty() {
            let mut combined = InternalResult::default();
            for m in &self.any_of {
                let result = m.validate(val, schema, pos);
                combined = pick_best_result(combined, result);
            }
            return combined;
        }

        // List validation
        if self.items.is_some() || !self.required_items.is_empty() {
            if val.scalar.is_some() || !val.map.is_empty() {
                return InternalResult::new(pos, Attempt::value(val, false, Some(matcher)));
            }

            let ok = val.list.len() >= self.required_items.len();
            let mut combined =
                InternalResult::new(pos, Attempt::value(val, ok, Some(matcher)));

            for (ix, entry) in val.list.iter().enumerate() {
                let key_pos = ResultPos::Key(entry.key.lno);
                combined = combined.append(key_pos, Attempt::key(entry, true, None));

                let value_pos = ResultPos::Value(entry.key.lno);
                if ix < self.required_items.len() {
                    let item_result = self.required_items[ix].validate(&entry.value, schema, value_pos);
                    combined = combined.append_all(item_result);
                } else if let Some(ref items) = self.items {
                    let item_result = items.validate(&entry.value, schema, value_pos);
                    combined = combined.append_all(item_result);
                } else {
                    combined = combined.append(key_pos, Attempt::key(entry, false, None));
                }
            }
            return combined;
        }

        // Map validation
        if !self.keys.is_empty() || !self.required_keys.is_empty() {
            if val.scalar.is_some() || !val.list.is_empty() {
                return InternalResult::new(pos, Attempt::value(val, false, Some(matcher)));
            }

            let mut seen: HashMap<String, bool> = HashMap::new();
            let mut seen_required: HashMap<usize, bool> = HashMap::new();
            let mut combined = InternalResult::default();

            'outer: for entry in &val.map {
                if *seen.get(&entry.key.content).unwrap_or(&false) {
                    combined = combined.append(
                        ResultPos::Key(entry.key.lno),
                        Attempt::key(entry, false, None),
                    );
                    continue;
                }
                seen.insert(entry.key.content.clone(), true);

                let key_val = ConlValue {
                    scalar: Some(entry.key.clone()),
                    ..Default::default()
                };
                let mut any_of: Vec<&Matcher> = Vec::new();
                let mut key_result = InternalResult::default();
                let key_pos = ResultPos::Key(entry.key.lno);

                for (i, (k, v)) in self.required_keys.iter().enumerate() {
                    let kr = k.validate(&key_val, schema, key_pos);
                    key_result = pick_best_result(key_result, kr.clone());
                    if kr.first_err == i32::MAX {
                        if *seen_required.get(&i).unwrap_or(&false) {
                            let mut attempt = Attempt::key(entry, false, None);
                            attempt.duplicate = Some(k.raw.clone());
                            combined = combined.append(key_pos, attempt);
                            continue 'outer;
                        }
                        seen_required.insert(i, true);
                        any_of.push(v);
                        break;
                    }
                }

                if any_of.is_empty() {
                    for (k, v) in &self.keys {
                        let kr = k.validate(&key_val, schema, key_pos);
                        if kr.first_err == i32::MAX {
                            any_of.push(v);
                        }
                        key_result = pick_best_result(key_result, kr);
                    }
                }

                combined = combined.append_all(key_result);
                if any_of.is_empty() {
                    continue;
                }

                let mut value_result = InternalResult::default();
                let value_pos = ResultPos::Value(entry.key.lno);
                for m in any_of {
                    let result = m.validate(&entry.value, schema, value_pos);
                    value_result = pick_best_result(value_result, result);
                }
                combined = combined.append_all(value_result);
            }

            let mut missing: Vec<&Matcher> = Vec::new();
            for (i, (k, _)) in self.required_keys.iter().enumerate() {
                if !*seen_required.get(&i).unwrap_or(&false) {
                    missing.push(k);
                }
            }

            let mut attempt = Attempt::value(val, missing.is_empty(), Some(matcher));
            attempt.missing_keys = missing.iter().map(|m| m.raw.clone()).collect();
            combined = combined.append(pos, attempt);

            return combined;
        }

        // Empty definition matches NoValue
        let matches = val.scalar.is_none() && val.list.is_empty() && val.map.is_empty();
        InternalResult::new(pos, Attempt::value(val, matches, Some(matcher)))
    }
}

/// SchemaError represents an error in parsing or validating a schema.
#[derive(Debug)]
pub struct SchemaError {
    pub lno: usize,
    pub msg: String,
}

impl SchemaError {
    fn new(lno: usize, msg: impl Into<String>) -> Self {
        Self {
            lno,
            msg: msg.into(),
        }
    }
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.lno > 0 {
            write!(f, "{}: {}", self.lno, self.msg)
        } else {
            write!(f, "{}", self.msg)
        }
    }
}

impl std::error::Error for SchemaError {}

impl From<SyntaxError> for SchemaError {
    fn from(err: SyntaxError) -> Self {
        SchemaError {
            lno: err.lno,
            msg: err.msg,
        }
    }
}

/// A ConlToken represents a parsed scalar value with line number information.
#[derive(Debug, Clone, Default)]
pub struct ConlToken {
    pub lno: usize,
    pub content: String,
    pub kind: TokenKind,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum TokenKind {
    #[default]
    Scalar,
    ListItem,
    MapKey,
}

/// A ConlValue represents a parsed CONL value that can be validated.
#[derive(Debug, Clone, Default)]
pub struct ConlValue {
    pub scalar: Option<ConlToken>,
    pub map: Vec<Entry>,
    pub list: Vec<Entry>,
}

/// An Entry in a map or list.
#[derive(Debug, Clone)]
pub struct Entry {
    pub key: ConlToken,
    pub value: ConlValue,
    pub parent_lno: usize,
}

/// Path segment to track position in document tree
#[derive(Debug, Clone)]
enum PathSegment {
    MapEntry(usize),
    ListEntry(usize),
}

fn parse_doc(input: &[u8]) -> ConlValue {
    let mut root = ConlValue::default();
    let mut path: Vec<PathSegment> = Vec::new();
    let mut lno_stack: Vec<usize> = vec![0];
    let no_value = ConlValue::default();
    let mut last_lno = 0;

    // Helper to get mutable reference at path
    fn get_at_path<'a>(root: &'a mut ConlValue, path: &[PathSegment]) -> &'a mut ConlValue {
        let mut current = root;
        for seg in path {
            match seg {
                PathSegment::MapEntry(idx) => {
                    current = &mut current.map[*idx].value;
                }
                PathSegment::ListEntry(idx) => {
                    current = &mut current.list[*idx].value;
                }
            }
        }
        current
    }

    for token_result in parse(input) {
        let token = match token_result {
            Ok(t) => t,
            Err(e) => {
                let current = get_at_path(&mut root, &path);
                if !current.map.is_empty() {
                    current.map.last_mut().unwrap().key.error = Some(e.msg);
                } else if !current.list.is_empty() {
                    current.list.last_mut().unwrap().key.error = Some(e.msg);
                }
                continue;
            }
        };

        let parent_lno = *lno_stack.last().unwrap_or(&0);

        match &token {
            Token::MapKey(lno, _) => {
                last_lno = *lno;
                let content = token.unescape().unwrap_or(Cow::Borrowed("")).to_string();
                let current = get_at_path(&mut root, &path);
                current.map.push(Entry {
                    key: ConlToken {
                        lno: *lno,
                        content,
                        kind: TokenKind::MapKey,
                        error: None,
                    },
                    value: no_value.clone(),
                    parent_lno,
                });
            }
            Token::ListItem(lno) => {
                last_lno = *lno;
                let current = get_at_path(&mut root, &path);
                current.list.push(Entry {
                    key: ConlToken {
                        lno: *lno,
                        content: String::new(),
                        kind: TokenKind::ListItem,
                        error: None,
                    },
                    value: no_value.clone(),
                    parent_lno,
                });
            }
            Token::Value(lno, _) | Token::MultilineValue(lno, _, _) => {
                let content = token.unescape().unwrap_or(Cow::Borrowed("")).to_string();
                let current = get_at_path(&mut root, &path);
                let scalar = ConlToken {
                    lno: *lno,
                    content,
                    kind: TokenKind::Scalar,
                    error: None,
                };
                if !current.map.is_empty() {
                    current.map.last_mut().unwrap().value = ConlValue {
                        scalar: Some(scalar),
                        ..Default::default()
                    };
                } else if !current.list.is_empty() {
                    current.list.last_mut().unwrap().value = ConlValue {
                        scalar: Some(scalar),
                        ..Default::default()
                    };
                }
            }
            Token::Indent(_) => {
                let current = get_at_path(&mut root, &path);
                if !current.map.is_empty() {
                    let idx = current.map.len() - 1;
                    current.map[idx].value = ConlValue::default();
                    path.push(PathSegment::MapEntry(idx));
                    lno_stack.push(last_lno);
                } else if !current.list.is_empty() {
                    let idx = current.list.len() - 1;
                    current.list[idx].value = ConlValue::default();
                    path.push(PathSegment::ListEntry(idx));
                    lno_stack.push(last_lno);
                }
            }
            Token::Outdent(_) => {
                path.pop();
                lno_stack.pop();
            }
            Token::NoValue(_) | Token::MultilineHint(_, _) | Token::Comment(_, _) | Token::Newline(_) => {}
        }
    }

    root
}

impl Schema {
    /// Parse a schema from the given input.
    ///
    /// An error is returned if the input is not valid CONL,
    /// or if the schema contains references to definitions that don't exist,
    /// invalid regular expressions, or circular references.
    pub fn parse(input: &[u8]) -> Result<Self, SchemaError> {
        let doc = parse_doc(input);

        let mut root: Option<Matcher> = None;
        let mut definitions: HashMap<String, Definition> = HashMap::new();

        // Parse root
        for entry in &doc.map {
            if entry.key.content == "root" {
                if let Some(ref scalar) = entry.value.scalar {
                    root = Some(Matcher::from_str(&scalar.content)?);
                }
            } else if entry.key.content == "definitions" {
                // Parse definitions
                for def_entry in &entry.value.map {
                    let name = def_entry.key.content.clone();
                    let mut def = Definition {
                        name: name.clone(),
                        ..Default::default()
                    };
                    parse_definition(&def_entry.value, &mut def)?;
                    definitions.insert(name, def);
                }
            } else if entry.key.content == "schema" {
                // Allow schema key, ignore it
            }
        }

        let root = root.ok_or_else(|| SchemaError::new(0, "missing root"))?;

        let schema = Schema { root, definitions };

        // Resolve all references
        let mut resolved = std::collections::HashSet::new();
        schema.root.resolve(&schema, &mut vec![], &mut resolved)?;

        Ok(schema)
    }

    /// Validate the input against the schema.
    pub fn validate(&self, input: &[u8]) -> SchemaResult<'_> {
        let doc = parse_doc(input);
        let result = self.root.validate(&doc, self, ResultPos::Value(0));
        SchemaResult {
            raw: result.raw,
            first_err: result.first_err,
            doc,
            schema: self,
        }
    }

    /// Returns a schema that validates any CONL document.
    pub fn any() -> Self {
        Schema::parse(
            br#"
root = <any>
definitions
  any
    any of
      = <map>
      = <list>
      = .*
  list
    items = <any>
  map
    keys
      .* = <any>
"#,
        )
        .expect("any schema should parse")
    }
}

fn parse_definition(val: &ConlValue, def: &mut Definition) -> Result<(), SchemaError> {
    for entry in &val.map {
        match entry.key.content.as_str() {
            "docs" => {
                if let Some(ref scalar) = entry.value.scalar {
                    def.docs = scalar.content.clone();
                }
            }
            "scalar" => {
                def.scalar = Some(parse_matcher(&entry.value)?);
            }
            "any of" => {
                for item in &entry.value.list {
                    def.any_of.push(parse_matcher(&item.value)?);
                }
            }
            "keys" => {
                for kv in &entry.value.map {
                    let key_matcher = Matcher::from_str(&kv.key.content)?;
                    let val_matcher = parse_matcher(&kv.value)?;
                    // Copy docs from value matcher to key matcher
                    let key_docs = val_matcher.docs.clone();
                    let mut key_matcher_with_docs = key_matcher;
                    key_matcher_with_docs.docs = key_docs;
                    def.keys.push((key_matcher_with_docs, val_matcher));
                }
            }
            "required keys" => {
                for kv in &entry.value.map {
                    let key_matcher = Matcher::from_str(&kv.key.content)?;
                    let val_matcher = parse_matcher(&kv.value)?;
                    let key_docs = val_matcher.docs.clone();
                    let mut key_matcher_with_docs = key_matcher;
                    key_matcher_with_docs.docs = key_docs;
                    def.required_keys.push((key_matcher_with_docs, val_matcher));
                }
            }
            "items" => {
                def.items = Some(parse_matcher(&entry.value)?);
            }
            "required items" => {
                for item in &entry.value.list {
                    def.required_items.push(parse_matcher(&item.value)?);
                }
            }
            _ => {
                // Unknown key, ignore for forward compatibility
            }
        }
    }
    Ok(())
}

fn parse_matcher(val: &ConlValue) -> Result<Matcher, SchemaError> {
    if let Some(ref scalar) = val.scalar {
        return Matcher::from_str(&scalar.content);
    }

    // Map form with docs
    let mut matcher = Matcher::default();
    for entry in &val.map {
        match entry.key.content.as_str() {
            "matches" => {
                if let Some(ref scalar) = entry.value.scalar {
                    let m = Matcher::from_str(&scalar.content)?;
                    matcher.pattern = m.pattern;
                    matcher.reference = m.reference;
                    matcher.raw = m.raw;
                }
            }
            "docs" => {
                if let Some(ref scalar) = entry.value.scalar {
                    matcher.docs = scalar.content.clone();
                }
            }
            _ => {}
        }
    }

    if matcher.pattern.is_none() && matcher.reference.is_none() {
        return Err(SchemaError::new(0, "missing matcher"));
    }

    Ok(matcher)
}

// Result types for validation

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResultPos {
    Key(usize),
    Value(usize),
}

impl ResultPos {
    fn lno(&self) -> usize {
        match self {
            ResultPos::Key(lno) => *lno,
            ResultPos::Value(lno) => *lno,
        }
    }

    fn is_key(&self) -> bool {
        matches!(self, ResultPos::Key(_))
    }
}

#[derive(Debug, Clone)]
struct Attempt {
    matcher: Option<Matcher>,
    val: ConlValue,
    ok: bool,
    missing_keys: Vec<String>,
    duplicate: Option<String>,
    parent_lno: usize,
}

impl Attempt {
    fn value(val: &ConlValue, ok: bool, matcher: Option<&Matcher>) -> Self {
        Attempt {
            matcher: matcher.cloned(),
            val: val.clone(),
            ok,
            missing_keys: Vec::new(),
            duplicate: None,
            parent_lno: 0,
        }
    }

    fn key(entry: &Entry, ok: bool, matcher: Option<&Matcher>) -> Self {
        Attempt {
            matcher: matcher.cloned(),
            val: ConlValue {
                scalar: Some(entry.key.clone()),
                ..Default::default()
            },
            ok,
            missing_keys: Vec::new(),
            duplicate: None,
            parent_lno: entry.parent_lno,
        }
    }
}

#[derive(Debug, Clone)]
struct InternalResult {
    raw: HashMap<ResultPos, Vec<Attempt>>,
    first_err: i32,
    err_count: i32,
}

impl Default for InternalResult {
    fn default() -> Self {
        InternalResult {
            raw: HashMap::new(),
            first_err: i32::MAX,  // No errors by default
            err_count: 0,
        }
    }
}

impl InternalResult {
    fn new(pos: ResultPos, attempt: Attempt) -> Self {
        let mut raw = HashMap::new();
        let (first_err, err_count) = if attempt.ok {
            (i32::MAX, 0)
        } else {
            (pos.lno() as i32, 1)
        };
        raw.insert(pos, vec![attempt]);
        InternalResult {
            raw,
            first_err,
            err_count,
        }
    }

    fn append(mut self, pos: ResultPos, attempt: Attempt) -> Self {
        if !attempt.ok && !self.raw.contains_key(&pos) {
            self.err_count += 1;
        }
        self.raw.entry(pos).or_default().push(attempt.clone());
        if !attempt.ok
            && ((pos.lno() as i32) > self.first_err || self.first_err == i32::MAX)
        {
            self.first_err = pos.lno() as i32;
        }
        self
    }

    fn append_all(mut self, other: InternalResult) -> Self {
        for (pos, attempts) in other.raw {
            for attempt in attempts {
                self = self.append(pos, attempt);
            }
        }
        self
    }
}

fn pick_best_result(r1: InternalResult, r2: InternalResult) -> InternalResult {
    if r1.raw.is_empty() {
        return r2;
    }
    if r2.raw.is_empty() {
        return r1;
    }
    if r1.first_err > r2.first_err {
        return r1;
    }
    if r2.first_err > r1.first_err {
        return r2;
    }
    if r1.err_count < r2.err_count {
        return r1;
    }
    if r2.err_count < r1.err_count {
        return r2;
    }
    r1.append_all(r2)
}

/// A ValidationError represents a single validation error.
#[derive(Debug, Clone)]
pub struct ValidationError {
    msg: String,
    pos: ResultPos,
}

impl ValidationError {
    /// Returns the line number (1-based) on which the error occurred.
    pub fn lno(&self) -> usize {
        if matches!(self.pos, ResultPos::Value(0)) {
            1
        } else {
            self.pos.lno()
        }
    }

    /// Returns a human-readable description of the problem.
    pub fn msg(&self) -> &str {
        &self.msg
    }

    /// Returns the rune range (0-based start, end) for the error.
    pub fn rune_range(&self, line: &str) -> (usize, usize) {
        let (start_key, end_key, start_value, end_value, _) = split_line(line);
        if matches!(self.pos, ResultPos::Value(0)) {
            return (start_key, end_value);
        }
        if self.pos.is_key() || start_value == end_value {
            (start_key, end_key)
        } else {
            (start_value, end_value)
        }
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.lno(), self.msg)
    }
}

impl std::error::Error for ValidationError {}

fn split_line(line: &str) -> (usize, usize, usize, usize, usize) {
    let trimmed = line.trim_start_matches(|c| c == ' ' || c == '\t');
    let start_key = line.len() - trimmed.len();

    // Replace quoted strings with placeholder
    let quoted_re = Regex::new(r#"^"(?:[^\\"]|\\.)*""#).unwrap();
    let trimmed_replaced = quoted_re.replace(trimmed, |caps: &regex::Captures| {
        "a".repeat(caps[0].len())
    });

    let (end_key, start_value) = if trimmed_replaced.starts_with('=') {
        (start_key + 1, start_key + 1)
    } else if let Some(found) = trimmed_replaced.find(|c| c == '=' || c == ';') {
        let end_key = start_key
            + trimmed_replaced[..found]
                .trim_end_matches(|c| c == ' ' || c == '\t')
                .len();
        let start_value = if trimmed_replaced.chars().nth(found) == Some('=') {
            start_key + found + 1
        } else {
            start_key + found
        };
        (end_key, start_value)
    } else {
        let end_key = start_key
            + trimmed_replaced
                .trim_end_matches(|c| c == ' ' || c == '\t')
                .len();
        (end_key, line.len())
    };

    let value_half = &line[start_value..];
    let trimmed_value = value_half.trim_start_matches(|c| c == ' ' || c == '\t');
    let start_value = start_value + (value_half.len() - trimmed_value.len());

    let trimmed_value_replaced = quoted_re.replace(trimmed_value, |caps: &regex::Captures| {
        "a".repeat(caps[0].len())
    });

    let (end_value, start_comment) = if let Some(found) = trimmed_value_replaced.find(';') {
        let end_value = start_value
            + trimmed_value_replaced[..found]
                .trim_end_matches(|c| c == ' ' || c == '\t')
                .len();
        (end_value, start_value + found)
    } else {
        let end_value = start_value
            + trimmed_value_replaced
                .trim_end_matches(|c| c == ' ' || c == '\t')
                .len();
        (end_value, line.len())
    };

    (start_key, end_key, start_value, end_value, start_comment)
}

fn join_with_or(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        _ => {
            let last = &items[items.len() - 1];
            let rest = &items[..items.len() - 1];
            format!("{} or {}", rest.join(", "), last)
        }
    }
}

fn validation_error(pos: ResultPos, attempts: &[Attempt]) -> Option<ValidationError> {
    let mut top_p = 0;
    let mut msg = String::new();
    let mut expected: Vec<String> = Vec::new();
    let mut missing_keys: Vec<String> = Vec::new();

    let add_error = |top_p: &mut i32, msg: &mut String, p: i32, new_msg: String| {
        if p > *top_p {
            *top_p = p;
            *msg = new_msg;
        }
    };

    for attempt in attempts {
        if attempt.ok {
            continue;
        }

        if let Some(ref scalar) = attempt.val.scalar {
            if let Some(ref error) = scalar.error {
                add_error(&mut top_p, &mut msg, 100, error.clone());
                continue;
            }
        }

        if attempt.matcher.is_none() {
            if let Some(ref scalar) = attempt.val.scalar {
                if scalar.kind == TokenKind::ListItem {
                    add_error(&mut top_p, &mut msg, 90, "unexpected list item".to_string());
                } else if let Some(ref dup) = attempt.duplicate {
                    add_error(&mut top_p, &mut msg, 90, format!("duplicate key {}", dup));
                } else {
                    add_error(&mut top_p, &mut msg, 90, format!("duplicate key {}", scalar.content));
                }
            }
            continue;
        }

        if !attempt.missing_keys.is_empty() {
            for k in &attempt.missing_keys {
                missing_keys.extend(suggestions_from_pattern(k, true));
            }
            continue;
        }

        if let Some(ref matcher) = attempt.matcher {
            if matcher.reference.is_none() {
                // Pattern matcher
                if pos.is_key() {
                    if let Some(ref scalar) = attempt.val.scalar {
                        add_error(&mut top_p, &mut msg, 80, format!("unexpected key {}", scalar.content));
                    }
                    continue;
                }
                if attempt.val.scalar.is_none() {
                    expected.push("any scalar".to_string());
                } else {
                    expected.extend(suggestions_from_pattern(&matcher.raw, true));
                }
            } else {
                // Definition reference - we'd need to check the definition type
                // For now, add generic expected messages
                expected.push("valid value".to_string());
            }
        }
    }

    if top_p > 50 {
        return Some(ValidationError { msg, pos });
    }
    if !expected.is_empty() {
        expected.sort();
        expected.dedup();
        return Some(ValidationError {
            msg: format!("expected {}", join_with_or(&expected)),
            pos,
        });
    }
    if !missing_keys.is_empty() {
        missing_keys.sort();
        missing_keys.dedup();
        return Some(ValidationError {
            msg: format!("missing required key {}", join_with_or(&missing_keys)),
            pos,
        });
    }
    if !msg.is_empty() {
        return Some(ValidationError { msg, pos });
    }
    None
}

/// A Suggestion is returned by suggested_keys or suggested_values.
#[derive(Debug, Clone)]
pub struct Suggestion {
    pub value: String,
    pub docs: String,
}

fn suggestions_from_pattern(pattern: &str, raw: bool) -> Vec<String> {
    if pattern.contains(|c| ".\\[](){}^$?*+".contains(c)) {
        if raw {
            return vec![pattern.to_string()];
        }
        return vec![];
    }
    pattern.split('|').map(|s| s.to_string()).collect()
}

/// A SchemaResult is produced when validating a document against a Schema.
pub struct SchemaResult<'a> {
    raw: HashMap<ResultPos, Vec<Attempt>>,
    first_err: i32,
    #[allow(dead_code)] // Used for suggested_keys/suggested_values
    doc: ConlValue,
    schema: &'a Schema,
}

impl<'a> SchemaResult<'a> {
    /// Returns true if the document matches the schema.
    pub fn is_valid(&self) -> bool {
        self.first_err == i32::MAX
    }

    /// Returns a list of validation errors.
    pub fn errors(&self) -> Vec<ValidationError> {
        if self.first_err == i32::MAX {
            return vec![];
        }
        let mut result: Vec<ValidationError> = Vec::new();
        for (pos, attempts) in &self.raw {
            if let Some(ve) = validation_error(*pos, attempts) {
                result.push(ve);
            }
        }
        result.sort_by(|a, b| {
            if a.lno() == b.lno() {
                (a.pos.is_key() as i32).cmp(&(b.pos.is_key() as i32))
            } else {
                a.lno().cmp(&b.lno())
            }
        });
        result
    }

    /// Returns possible keys for the map at the given line (0 for root).
    pub fn suggested_keys(&self, line: usize) -> Vec<Suggestion> {
        let pos = ResultPos::Value(line);
        let mut possible: Vec<&Matcher> = Vec::new();
        let mut list_allowed = false;
        let mut val: Option<&ConlValue> = None;

        if let Some(attempts) = self.raw.get(&pos) {
            for attempt in attempts {
                if let Some(ref matcher) = attempt.matcher {
                    if let Some(def) = matcher.get_definition(self.schema) {
                        val = Some(&attempt.val);
                        for (k, _) in &def.keys {
                            possible.push(k);
                        }
                        for (k, _) in &def.required_keys {
                            possible.push(k);
                        }
                        if def.items.is_some() || !def.required_items.is_empty() {
                            list_allowed = true;
                        }
                    }
                }
            }
        }

        // Filter out already-present keys
        let mut filtered: Vec<&Matcher> = Vec::new();
        if let Some(val) = val {
            'outer: for p in &possible {
                for entry in &val.map {
                    let key_val = ConlValue {
                        scalar: Some(entry.key.clone()),
                        ..Default::default()
                    };
                    let kr = p.validate(&key_val, self.schema, ResultPos::Key(entry.key.lno));
                    if kr.err_count == 0 {
                        continue 'outer;
                    }
                }
                filtered.push(*p);
            }
        } else {
            filtered = possible;
        }

        let mut results: Vec<Suggestion> = Vec::new();
        for p in filtered {
            results.extend(p.suggested_values(self.schema));
        }
        if list_allowed {
            results.push(Suggestion {
                value: "=".to_string(),
                docs: String::new(),
            });
        }
        results.sort_by(|a, b| a.value.cmp(&b.value));
        results.dedup_by(|a, b| a.value == b.value);
        results
    }

    /// Returns possible values for the key at the given line.
    pub fn suggested_values(&self, line: usize) -> Vec<Suggestion> {
        let key_pos = ResultPos::Key(line);
        let mut parent_lno = 0;
        let mut key: Option<&ConlValue> = None;

        if let Some(attempts) = self.raw.get(&key_pos) {
            for attempt in attempts {
                parent_lno = attempt.parent_lno;
                key = Some(&attempt.val);
                break;
            }
        }

        let key = match key {
            Some(k) => k,
            None => return vec![],
        };

        let mut possible: Vec<&Matcher> = Vec::new();
        let value_pos = ResultPos::Value(parent_lno);

        if let Some(attempts) = self.raw.get(&value_pos) {
            for attempt in attempts {
                if let Some(ref matcher) = attempt.matcher {
                    if let Some(def) = matcher.get_definition(self.schema) {
                        if key.scalar.as_ref().map(|s| s.kind == TokenKind::ListItem).unwrap_or(false) {
                            // List item
                            for (ix, e) in attempt.val.list.iter().enumerate() {
                                if let Some(ref k) = key.scalar {
                                    if e.key.lno == k.lno {
                                        if ix < def.required_items.len() {
                                            possible.push(&def.required_items[ix]);
                                        } else if let Some(ref items) = def.items {
                                            possible.push(items);
                                        }
                                    }
                                }
                            }
                            continue;
                        }

                        for (k, v) in &def.required_keys {
                            let kr = k.validate(key, self.schema, key_pos);
                            if kr.err_count == 0 {
                                possible.push(v);
                            }
                        }
                        for (k, v) in &def.keys {
                            let kr = k.validate(key, self.schema, key_pos);
                            if kr.err_count == 0 {
                                possible.push(v);
                            }
                        }
                    }
                }
            }
        }

        let mut results: Vec<Suggestion> = Vec::new();
        for p in possible {
            results.extend(p.suggested_values(self.schema));
        }
        results.sort_by(|a, b| a.value.cmp(&b.value));
        results.dedup_by(|a, b| a.value == b.value);
        results
    }

    /// Returns docs for the key at the given line.
    pub fn docs_for_key(&self, line: usize) -> Option<String> {
        let pos = ResultPos::Key(line);
        if let Some(attempts) = self.raw.get(&pos) {
            for attempt in attempts {
                if let Some(ref matcher) = attempt.matcher {
                    if attempt.ok && !matcher.docs.is_empty() {
                        return Some(matcher.docs.clone());
                    }
                }
            }
        }
        None
    }

    /// Returns docs for the value at the given line.
    pub fn docs_for_value(&self, line: usize) -> Option<String> {
        let pos = ResultPos::Value(line);
        if let Some(attempts) = self.raw.get(&pos) {
            for attempt in attempts {
                if let Some(ref matcher) = attempt.matcher {
                    if attempt.ok && !matcher.docs.is_empty() {
                        return Some(matcher.docs.clone());
                    }
                }
            }
        }
        None
    }
}

/// Validate a CONL document with optional schema loading.
/// If the document contains a top-level 'schema' key, its value is passed to load.
pub fn validate_with_loader<F>(input: &[u8], load: F) -> Result<(bool, Vec<ValidationError>), SchemaError>
where
    F: FnOnce(&str) -> Result<Option<Schema>, SchemaError>,
{
    let doc = parse_doc(input);
    let mut schema_name = String::new();

    for entry in &doc.map {
        if entry.key.content == "schema" {
            if let Some(ref scalar) = entry.value.scalar {
                schema_name = scalar.content.clone();
                break;
            }
        }
    }

    let schema = match load(&schema_name)? {
        Some(s) => s,
        None => Schema::any(),
    };

    let result = schema.validate(input);
    Ok((result.is_valid(), result.errors()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_any_schema() {
        let schema = Schema::any();
        let input = br#"
key = value
list
  = item1
  = item2
nested
  inner = data
"#;
        let result = schema.validate(input);
        assert!(result.is_valid());
    }

    #[test]
    fn test_basic_schema() {
        let schema = Schema::parse(
            br#"
root = <config>
definitions
  config
    required keys
      name = .*
    keys
      port = \d+
"#,
        )
        .unwrap();

        // Valid document
        let result = schema.validate(b"name = test\nport = 8080");
        assert!(result.is_valid());

        // Missing required key
        let result = schema.validate(b"port = 8080");
        assert!(!result.is_valid());
        assert!(!result.errors().is_empty());
    }

    #[test]
    fn test_list_schema() {
        let schema = Schema::parse(
            br#"
root = <list>
definitions
  list
    items = \d+
"#,
        )
        .unwrap();

        let result = schema.validate(b"= 1\n= 2\n= 3");
        assert!(result.is_valid());

        let result = schema.validate(b"= 1\n= abc\n= 3");
        assert!(!result.is_valid());
    }

    #[test]
    fn test_any_of() {
        // Test any_of with a list that contains values matching one of the patterns
        let schema = Schema::parse(
            br#"
root = <list>
definitions
  list
    items = <value>
  value
    any of
      = true|false
      = \d+
"#,
        )
        .unwrap();

        let result = schema.validate(b"= true");
        assert!(result.is_valid());

        let result = schema.validate(b"= 42");
        assert!(result.is_valid());

        let result = schema.validate(b"= hello");
        assert!(!result.is_valid());
    }

    #[test]
    fn test_nested_schema() {
        let schema = Schema::parse(
            br#"
root = <server>
definitions
  server
    required keys
      type = server
    keys
      listen = <addr>
  addr
    keys
      host = .*
      port = \d+
"#,
        )
        .unwrap();

        let result = schema.validate(
            br#"type = server
listen
  host = localhost
  port = 8080
"#,
        );
        assert!(result.is_valid());
    }

    #[test]
    fn test_circular_reference_detection() {
        let result = Schema::parse(
            br#"
root = <a>
definitions
  a
    scalar = <b>
  b
    scalar = <a>
"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_undefined_reference() {
        let result = Schema::parse(
            br#"
root = <undefined>
definitions
  defined
    scalar = .*
"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_split_line() {
        let (sk, ek, sv, ev, sc) = split_line("  key = value ; comment");
        assert_eq!(sk, 2);
        assert_eq!(ek, 5);
        assert_eq!(sv, 8);
        assert_eq!(ev, 13);
        assert_eq!(sc, 14);
    }
}
