//! Serde serializer for CONL format.
//!
//! This module provides a serde `Serializer` implementation that outputs CONL format.
//!
//! # Example
//!
//! ```
//! use serde::Serialize;
//!
//! #[derive(Serialize)]
//! struct Config {
//!     name: String,
//!     port: u16,
//!     enabled: bool,
//! }
//!
//! let config = Config {
//!     name: "my-app".to_string(),
//!     port: 8080,
//!     enabled: true,
//! };
//!
//! let conl = conl::ser::to_string(&config).unwrap();
//! ```

use serde::ser::{self, Serialize};
use std::fmt::{self, Display};

/// Error type for serialization errors.
#[derive(Debug)]
pub struct Error {
    msg: String,
}

impl Error {
    fn new(msg: impl Into<String>) -> Self {
        Error { msg: msg.into() }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.msg)
    }
}

impl std::error::Error for Error {}

impl ser::Error for Error {
    fn custom<T: Display>(msg: T) -> Self {
        Error::new(msg.to_string())
    }
}

/// Serialize a value to a CONL string.
///
/// # Example
///
/// ```
/// use serde::Serialize;
///
/// #[derive(Serialize)]
/// struct Point {
///     x: i32,
///     y: i32,
/// }
///
/// let point = Point { x: 1, y: 2 };
/// let conl = conl::ser::to_string(&point).unwrap();
/// assert_eq!(conl, "x = 1\ny = 2\n");
/// ```
pub fn to_string<T: Serialize>(value: &T) -> Result<String, Error> {
    let mut serializer = Serializer::new();
    value.serialize(&mut serializer)?;
    Ok(serializer.output)
}

/// The CONL serializer.
pub struct Serializer {
    output: String,
    indent: String,
    /// Whether we're in inline mode (for top-level scalars)
    inline: bool,
}

impl Serializer {
    fn new() -> Self {
        Serializer {
            output: String::new(),
            indent: String::new(),
            inline: false,
        }
    }

    fn write_indent(&mut self) {
        if !self.indent.is_empty() {
            self.output.push_str(&self.indent);
        }
    }

    fn increase_indent(&mut self) {
        self.indent.push_str("  ");
    }

    fn decrease_indent(&mut self) {
        if self.indent.len() >= 2 {
            self.indent.truncate(self.indent.len() - 2);
        }
    }
}

/// Check if a string value needs quoting in CONL.
fn needs_quoting(s: &str) -> bool {
    if s.is_empty() {
        return true; // Empty string needs quotes: ""
    }

    let bytes = s.as_bytes();

    // Leading or trailing whitespace
    if bytes.first().map_or(false, |&b| b == b' ' || b == b'\t')
        || bytes.last().map_or(false, |&b| b == b' ' || b == b'\t')
    {
        return true;
    }

    // Contains special characters
    for c in s.chars() {
        match c {
            ';' | '=' | '\n' | '\r' | '"' | '\\' => return true,
            _ => {}
        }
    }

    false
}

/// Check if a key needs quoting in CONL.
fn key_needs_quoting(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }

    let bytes = s.as_bytes();

    // Leading or trailing whitespace
    if bytes.first().map_or(false, |&b| b == b' ' || b == b'\t')
        || bytes.last().map_or(false, |&b| b == b' ' || b == b'\t')
    {
        return true;
    }

    // Contains special characters
    for c in s.chars() {
        match c {
            ';' | '=' | '\n' | '\r' | '"' | '\\' => return true,
            _ => {}
        }
    }

    false
}

/// Escape a string value for CONL output.
fn escape_string(s: &str) -> String {
    if !needs_quoting(s) {
        return s.to_string();
    }

    let mut output = String::with_capacity(s.len() + 2);
    output.push('"');
    for c in s.chars() {
        match c {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            _ if c.is_control() => {
                output.push_str(&format!("\\{{{:X}}}", c as u32));
            }
            _ => output.push(c),
        }
    }
    output.push('"');
    output
}

/// Escape a key for CONL output.
fn escape_key(s: &str) -> String {
    if !key_needs_quoting(s) {
        return s.to_string();
    }

    let mut output = String::with_capacity(s.len() + 2);
    output.push('"');
    for c in s.chars() {
        match c {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            _ if c.is_control() => {
                output.push_str(&format!("\\{{{:X}}}", c as u32));
            }
            _ => output.push(c),
        }
    }
    output.push('"');
    output
}

impl<'a> ser::Serializer for &'a mut Serializer {
    type Ok = ();
    type Error = Error;

    type SerializeSeq = SeqSerializer<'a>;
    type SerializeTuple = SeqSerializer<'a>;
    type SerializeTupleStruct = SeqSerializer<'a>;
    type SerializeTupleVariant = SeqSerializer<'a>;
    type SerializeMap = MapSerializer<'a>;
    type SerializeStruct = StructSerializer<'a>;
    type SerializeStructVariant = StructSerializer<'a>;

    fn serialize_bool(self, v: bool) -> Result<Self::Ok, Self::Error> {
        self.output.push_str(if v { "true" } else { "false" });
        if !self.inline {
            self.output.push('\n');
        }
        Ok(())
    }

    fn serialize_i8(self, v: i8) -> Result<Self::Ok, Self::Error> {
        self.serialize_i64(v as i64)
    }

    fn serialize_i16(self, v: i16) -> Result<Self::Ok, Self::Error> {
        self.serialize_i64(v as i64)
    }

    fn serialize_i32(self, v: i32) -> Result<Self::Ok, Self::Error> {
        self.serialize_i64(v as i64)
    }

    fn serialize_i64(self, v: i64) -> Result<Self::Ok, Self::Error> {
        self.output.push_str(&v.to_string());
        if !self.inline {
            self.output.push('\n');
        }
        Ok(())
    }

    fn serialize_u8(self, v: u8) -> Result<Self::Ok, Self::Error> {
        self.serialize_u64(v as u64)
    }

    fn serialize_u16(self, v: u16) -> Result<Self::Ok, Self::Error> {
        self.serialize_u64(v as u64)
    }

    fn serialize_u32(self, v: u32) -> Result<Self::Ok, Self::Error> {
        self.serialize_u64(v as u64)
    }

    fn serialize_u64(self, v: u64) -> Result<Self::Ok, Self::Error> {
        self.output.push_str(&v.to_string());
        if !self.inline {
            self.output.push('\n');
        }
        Ok(())
    }

    fn serialize_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
        self.serialize_f64(v as f64)
    }

    fn serialize_f64(self, v: f64) -> Result<Self::Ok, Self::Error> {
        self.output.push_str(&v.to_string());
        if !self.inline {
            self.output.push('\n');
        }
        Ok(())
    }

    fn serialize_char(self, v: char) -> Result<Self::Ok, Self::Error> {
        self.serialize_str(&v.to_string())
    }

    fn serialize_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
        self.output.push_str(&escape_string(v));
        if !self.inline {
            self.output.push('\n');
        }
        Ok(())
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<Self::Ok, Self::Error> {
        use ser::SerializeSeq;
        let mut seq = self.serialize_seq(Some(v.len()))?;
        for byte in v {
            seq.serialize_element(byte)?;
        }
        seq.end()
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        // NoValue - just a newline after the key/list item marker
        if !self.inline {
            self.output.push('\n');
        }
        Ok(())
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        self.serialize_none()
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        self.serialize_none()
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        self.serialize_str(variant)
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        // Serialize as a map with the variant name as key
        self.output.push_str(&escape_key(variant));
        self.output.push_str(" =");

        // Check if value is a compound type
        let mut probe = Serializer::new();
        probe.inline = true;
        value.serialize(&mut probe)?;

        // Check if the probe output contains nested structure indicators
        if probe.output.contains('\n') || probe.output.is_empty() {
            // Compound type - needs indentation
            self.output.push('\n');
            self.increase_indent();
            self.write_indent();
            value.serialize(&mut *self)?;
            self.decrease_indent();
        } else {
            // Simple type
            self.output.push(' ');
            self.inline = true;
            value.serialize(&mut *self)?;
            self.inline = false;
            self.output.push('\n');
        }
        Ok(())
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(SeqSerializer {
            ser: self,
            first: true,
        })
    }

    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        self.output.push_str(&escape_key(variant));
        self.output.push('\n');
        self.increase_indent();
        Ok(SeqSerializer {
            ser: self,
            first: true,
        })
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(MapSerializer {
            ser: self,
            first: true,
        })
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(StructSerializer {
            ser: self,
            first: true,
        })
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        self.output.push_str(&escape_key(variant));
        self.output.push('\n');
        self.increase_indent();
        Ok(StructSerializer {
            ser: self,
            first: true,
        })
    }
}

/// Serializer for sequences (lists/arrays).
pub struct SeqSerializer<'a> {
    ser: &'a mut Serializer,
    first: bool,
}

impl<'a> ser::SerializeSeq for SeqSerializer<'a> {
    type Ok = ();
    type Error = Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        // Check if value is compound
        let mut probe = Serializer::new();
        probe.inline = true;
        value.serialize(&mut probe)?;

        self.ser.write_indent();

        if probe.output.contains('\n') {
            // Compound value - no space after =
            self.ser.output.push_str("=\n");
            self.ser.increase_indent();
            value.serialize(&mut *self.ser)?;
            self.ser.decrease_indent();
        } else if probe.output.is_empty() {
            // null value
            self.ser.output.push_str("=\n");
        } else {
            // Simple value - space after =
            self.ser.output.push_str("= ");
            self.ser.inline = true;
            value.serialize(&mut *self.ser)?;
            self.ser.inline = false;
            self.ser.output.push('\n');
        }

        self.first = false;
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl<'a> ser::SerializeTuple for SeqSerializer<'a> {
    type Ok = ();
    type Error = Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        ser::SerializeSeq::end(self)
    }
}

impl<'a> ser::SerializeTupleStruct for SeqSerializer<'a> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        ser::SerializeSeq::end(self)
    }
}

impl<'a> ser::SerializeTupleVariant for SeqSerializer<'a> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        self.ser.decrease_indent();
        Ok(())
    }
}

/// Serializer for maps.
pub struct MapSerializer<'a> {
    ser: &'a mut Serializer,
    first: bool,
}

impl<'a> ser::SerializeMap for MapSerializer<'a> {
    type Ok = ();
    type Error = Error;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Self::Error> {
        self.ser.write_indent();

        // Serialize key to string
        let mut key_ser = Serializer::new();
        key_ser.inline = true;
        key.serialize(&mut key_ser)?;

        // Remove trailing newline if present
        let key_str = key_ser.output.trim_end_matches('\n');
        // The key was already escaped by serialize_str if it's a string,
        // but we need to re-escape for key context (= is special in keys too)
        // If it's already quoted, use as-is; otherwise apply key escaping
        if key_str.starts_with('"') && key_str.ends_with('"') {
            self.ser.output.push_str(key_str);
        } else {
            self.ser.output.push_str(&escape_key(key_str));
        }
        // Don't add " =" here, we'll do it in serialize_value based on value type

        self.first = false;
        Ok(())
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        // Check if value is compound
        let mut probe = Serializer::new();
        probe.inline = true;
        value.serialize(&mut probe)?;

        if probe.output.contains('\n') {
            // Compound value - no = sign
            self.ser.output.push('\n');
            self.ser.increase_indent();
            value.serialize(&mut *self.ser)?;
            self.ser.decrease_indent();
        } else if probe.output.is_empty() {
            // null value
            self.ser.output.push_str(" =\n");
        } else {
            // Simple value
            self.ser.output.push_str(" = ");
            self.ser.inline = true;
            value.serialize(&mut *self.ser)?;
            self.ser.inline = false;
            self.ser.output.push('\n');
        }

        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

/// Serializer for structs.
pub struct StructSerializer<'a> {
    ser: &'a mut Serializer,
    first: bool,
}

impl<'a> ser::SerializeStruct for StructSerializer<'a> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        // Check if value is compound
        let mut probe = Serializer::new();
        probe.inline = true;
        value.serialize(&mut probe)?;

        self.ser.write_indent();
        self.ser.output.push_str(&escape_key(key));

        if probe.output.contains('\n') {
            // Compound value - no = sign
            self.ser.output.push('\n');
            self.ser.increase_indent();
            value.serialize(&mut *self.ser)?;
            self.ser.decrease_indent();
        } else if probe.output.is_empty() {
            // null value
            self.ser.output.push_str(" =\n");
        } else {
            // Simple value
            self.ser.output.push_str(" = ");
            self.ser.inline = true;
            value.serialize(&mut *self.ser)?;
            self.ser.inline = false;
            self.ser.output.push('\n');
        }

        self.first = false;
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl<'a> ser::SerializeStructVariant for StructSerializer<'a> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        ser::SerializeStruct::serialize_field(self, key, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        self.ser.decrease_indent();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[test]
    fn test_simple_struct() {
        #[derive(Serialize)]
        struct Point {
            x: i32,
            y: i32,
        }

        let point = Point { x: 1, y: 2 };
        let result = to_string(&point).unwrap();
        assert_eq!(result, "x = 1\ny = 2\n");
    }

    #[test]
    fn test_nested_struct() {
        #[derive(Serialize)]
        struct Inner {
            value: String,
        }

        #[derive(Serialize)]
        struct Outer {
            name: String,
            inner: Inner,
        }

        let data = Outer {
            name: "test".to_string(),
            inner: Inner {
                value: "hello".to_string(),
            },
        };

        let result = to_string(&data).unwrap();
        assert_eq!(result, "name = test\ninner\n  value = hello\n");
    }

    #[test]
    fn test_vec() {
        #[derive(Serialize)]
        struct Data {
            items: Vec<String>,
        }

        let data = Data {
            items: vec!["a".to_string(), "b".to_string(), "c".to_string()],
        };

        let result = to_string(&data).unwrap();
        assert_eq!(result, "items\n  = a\n  = b\n  = c\n");
    }

    #[test]
    fn test_option_none() {
        #[derive(Serialize)]
        struct Data {
            value: Option<String>,
        }

        let data = Data { value: None };
        let result = to_string(&data).unwrap();
        assert_eq!(result, "value =\n");
    }

    #[test]
    fn test_option_some() {
        #[derive(Serialize)]
        struct Data {
            value: Option<String>,
        }

        let data = Data {
            value: Some("hello".to_string()),
        };
        let result = to_string(&data).unwrap();
        assert_eq!(result, "value = hello\n");
    }

    #[test]
    fn test_special_chars() {
        #[derive(Serialize)]
        struct Data {
            text: String,
        }

        let data = Data {
            text: "hello; world".to_string(),
        };
        let result = to_string(&data).unwrap();
        assert_eq!(result, "text = \"hello; world\"\n");
    }

    #[test]
    fn test_empty_string() {
        #[derive(Serialize)]
        struct Data {
            text: String,
        }

        let data = Data {
            text: "".to_string(),
        };
        let result = to_string(&data).unwrap();
        assert_eq!(result, "text = \"\"\n");
    }

    #[test]
    fn test_bool() {
        #[derive(Serialize)]
        struct Data {
            enabled: bool,
            disabled: bool,
        }

        let data = Data {
            enabled: true,
            disabled: false,
        };
        let result = to_string(&data).unwrap();
        assert_eq!(result, "enabled = true\ndisabled = false\n");
    }

    #[test]
    fn test_numbers() {
        #[derive(Serialize)]
        struct Data {
            int: i32,
            float: f64,
        }

        let data = Data {
            int: 42,
            float: 3.14,
        };
        let result = to_string(&data).unwrap();
        assert_eq!(result, "int = 42\nfloat = 3.14\n");
    }

    #[test]
    fn test_nested_vec() {
        #[derive(Serialize)]
        struct Item {
            name: String,
        }

        #[derive(Serialize)]
        struct Data {
            items: Vec<Item>,
        }

        let data = Data {
            items: vec![
                Item {
                    name: "first".to_string(),
                },
                Item {
                    name: "second".to_string(),
                },
            ],
        };

        let result = to_string(&data).unwrap();
        assert_eq!(result, "items\n  =\n    name = first\n  =\n    name = second\n");
    }

    #[test]
    fn test_escape_newline() {
        #[derive(Serialize)]
        struct Data {
            text: String,
        }

        let data = Data {
            text: "line1\nline2".to_string(),
        };
        let result = to_string(&data).unwrap();
        assert_eq!(result, "text = \"line1\\nline2\"\n");
    }

    #[test]
    fn test_escape_quotes() {
        #[derive(Serialize)]
        struct Data {
            text: String,
        }

        let data = Data {
            text: "say \"hello\"".to_string(),
        };
        let result = to_string(&data).unwrap();
        assert_eq!(result, "text = \"say \\\"hello\\\"\"\n");
    }

    #[test]
    fn test_key_with_special_chars() {
        use std::collections::HashMap;

        let mut map: HashMap<String, String> = HashMap::new();
        map.insert("key=value".to_string(), "test".to_string());

        let result = to_string(&map).unwrap();
        assert!(result.contains("\"key=value\" = test"));
    }

    #[test]
    fn test_unit_variant() {
        #[derive(Serialize)]
        #[allow(dead_code)]
        enum Status {
            Active,
            Inactive,
        }

        #[derive(Serialize)]
        struct Data {
            status: Status,
        }

        let data = Data {
            status: Status::Active,
        };
        let result = to_string(&data).unwrap();
        assert_eq!(result, "status = Active\n");
    }
}
