//! Minimal JCE (TARS) codec for Huya's danmaku websocket.
//!
//! Only the subset of the binary protocol that the Huya chat protocol uses is
//! implemented: structs, integer/bool/string fields, byte blobs (`SimpleList`)
//! and string maps. Wire format reference: Huya's web `lib.js` (`Taf`), see
//! `JceOutputStream`/`JceInputStream`. All integers are big-endian; a field is
//! prefixed by a head byte holding the field tag (high nibble) and type (low
//! nibble), with tags >= 15 stored in a follow-up byte.

use std::fmt;

const TYPE_INT8: u8 = 0;
const TYPE_INT16: u8 = 1;
const TYPE_INT32: u8 = 2;
const TYPE_INT64: u8 = 3;
const TYPE_STRING1: u8 = 6;
const TYPE_STRING4: u8 = 7;
const TYPE_MAP: u8 = 8;
const TYPE_LIST: u8 = 9;
const TYPE_STRUCT_BEGIN: u8 = 10;
const TYPE_STRUCT_END: u8 = 11;
const TYPE_ZERO: u8 = 12;
const TYPE_SIMPLELIST: u8 = 13;

#[derive(Debug)]
pub enum JceError {
    UnexpectedEof,
    TypeMismatch { tag: u8, ty: u8 },
    InvalidUtf8,
}

impl fmt::Display for JceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JceError::UnexpectedEof => write!(f, "unexpected end of JCE data"),
            JceError::TypeMismatch { tag, ty } => {
                write!(f, "type mismatch at tag {tag} (type {ty})")
            }
            JceError::InvalidUtf8 => write!(f, "JCE string is not valid UTF-8"),
        }
    }
}

/// Encodes fields into the JCE binary format.
pub struct JceWriter {
    buf: Vec<u8>,
}

impl Default for JceWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl JceWriter {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    fn head(&mut self, tag: u8, ty: u8) {
        if tag < 15 {
            self.buf.push((tag << 4) | ty);
        } else {
            self.buf.push(0xF0 | ty);
            self.buf.push(tag);
        }
    }

    /// Writes an integer with the smallest representable type, matching the
    /// reference encoder (`writeInt8`..`writeInt64` in lib.js).
    pub fn write_i64(&mut self, tag: u8, value: i64) {
        if value == 0 {
            self.head(tag, TYPE_ZERO);
        } else if (-128..=127).contains(&value) {
            self.head(tag, TYPE_INT8);
            self.buf.push(value as i8 as u8);
        } else if (-32768..=32767).contains(&value) {
            self.head(tag, TYPE_INT16);
            self.buf.extend_from_slice(&(value as i16).to_be_bytes());
        } else if (-2147483648..=2147483647).contains(&value) {
            self.head(tag, TYPE_INT32);
            self.buf.extend_from_slice(&(value as i32).to_be_bytes());
        } else {
            self.head(tag, TYPE_INT64);
            self.buf.extend_from_slice(&value.to_be_bytes());
        }
    }

    pub fn write_bool(&mut self, tag: u8, value: bool) {
        self.write_i64(tag, i64::from(value));
    }

    pub fn write_str(&mut self, tag: u8, value: &str) {
        let bytes = value.as_bytes();
        if bytes.len() > 255 {
            self.head(tag, TYPE_STRING4);
            self.buf
                .extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        } else {
            self.head(tag, TYPE_STRING1);
            self.buf.push(bytes.len() as u8);
        }
        self.buf.extend_from_slice(bytes);
    }

    /// Writes a byte blob (`SimpleList`).
    pub fn write_bytes(&mut self, tag: u8, value: &[u8]) {
        self.head(tag, TYPE_SIMPLELIST);
        self.head(0, TYPE_INT8);
        self.write_i64(0, value.len() as i64);
        self.buf.extend_from_slice(value);
    }

    /// Writes a `map<string, bytes>` entry set; empty maps are valid.
    pub fn write_string_bytes_map(&mut self, tag: u8, entries: &[(&str, &[u8])]) {
        self.head(tag, TYPE_MAP);
        self.write_i64(0, entries.len() as i64);
        for (key, value) in entries {
            self.write_str(0, key);
            self.write_bytes(1, value);
        }
    }

    /// Writes a nested struct: `STRUCTBEGIN`, the fields, `STRUCTEND`.
    pub fn write_struct(&mut self, tag: u8, fields: impl FnOnce(&mut JceWriter)) {
        self.head(tag, TYPE_STRUCT_BEGIN);
        fields(self);
        self.head(0, TYPE_STRUCT_END);
    }
}

/// Decodes JCE binary data field by field.
pub struct JceReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> JceReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], JceError> {
        if self.remaining() < len {
            return Err(JceError::UnexpectedEof);
        }
        let slice = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Ok(slice)
    }

    /// Peeks the next field head without consuming it: `(tag, type)`.
    fn peek_head(&self) -> Result<(u8, u8), JceError> {
        let byte = *self.data.get(self.pos).ok_or(JceError::UnexpectedEof)?;
        let tag = byte >> 4;
        let ty = byte & 0x0F;
        if tag < 15 {
            Ok((tag, ty))
        } else {
            let tag = *self.data.get(self.pos + 1).ok_or(JceError::UnexpectedEof)?;
            Ok((tag, ty))
        }
    }

    fn read_head(&mut self) -> Result<(u8, u8), JceError> {
        let byte = *self.take(1)?.first().expect("take(1) returns one byte");
        let tag = byte >> 4;
        let ty = byte & 0x0F;
        if tag < 15 {
            Ok((tag, ty))
        } else {
            let tag = *self.take(1)?.first().expect("take(1) returns one byte");
            Ok((tag, ty))
        }
    }

    fn read_i64_value(&mut self, ty: u8) -> Result<i64, JceError> {
        match ty {
            TYPE_ZERO => Ok(0),
            TYPE_INT8 => Ok(i8::from_be_bytes(self.take(1)?.try_into().unwrap()) as i64),
            TYPE_INT16 => Ok(i16::from_be_bytes(self.take(2)?.try_into().unwrap()) as i64),
            TYPE_INT32 => Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()) as i64),
            TYPE_INT64 => Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap())),
            _ => Err(JceError::TypeMismatch { tag: 0, ty }),
        }
    }

    /// Skips the value of a field given its type.
    fn skip_field(&mut self, ty: u8) -> Result<(), JceError> {
        match ty {
            TYPE_INT8 => {
                self.take(1)?;
            }
            TYPE_INT16 => {
                self.take(2)?;
            }
            TYPE_INT32 => {
                self.take(4)?;
            }
            TYPE_INT64 => {
                self.take(8)?;
            }
            TYPE_STRING1 => {
                let len = u8::from_be_bytes(self.take(1)?.try_into().unwrap()) as usize;
                self.take(len)?;
            }
            TYPE_STRING4 => {
                let len = u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as usize;
                self.take(len)?;
            }
            TYPE_STRUCT_BEGIN => self.skip_to_struct_end()?,
            TYPE_STRUCT_END | TYPE_ZERO => {}
            TYPE_MAP => {
                let count = self.read_i64_value_at_tag(0)?;
                for _ in 0..count {
                    self.skip_field_at_tag()?;
                    self.skip_field_at_tag()?;
                }
            }
            TYPE_LIST => {
                let count = self.read_i64_value_at_tag(0)?;
                for _ in 0..count {
                    self.skip_field_at_tag()?;
                }
            }
            TYPE_SIMPLELIST => {
                // Head of the length field must be an int8-typed tag 0, then
                // the length value itself, then the raw bytes.
                let (_, head_ty) = self.read_head()?;
                if head_ty != TYPE_INT8 {
                    return Err(JceError::TypeMismatch {
                        tag: 0,
                        ty: head_ty,
                    });
                }
                let len = self.read_i64_value_at_tag(0)? as usize;
                self.take(len)?;
            }
            _ => return Err(JceError::TypeMismatch { tag: 0, ty }),
        }
        Ok(())
    }

    /// Reads the `tag 0` integer that heads map/list/simplelist values.
    fn read_i64_value_at_tag(&mut self, expect_tag: u8) -> Result<i64, JceError> {
        let (tag, ty) = self.read_head()?;
        if tag != expect_tag {
            return Err(JceError::TypeMismatch { tag, ty });
        }
        self.read_i64_value(ty)
    }

    fn skip_field_at_tag(&mut self) -> Result<(), JceError> {
        let (_, ty) = self.read_head()?;
        self.skip_field(ty)
    }

    fn skip_to_struct_end(&mut self) -> Result<(), JceError> {
        loop {
            let (_, ty) = self.read_head()?;
            if ty == TYPE_STRUCT_END {
                return Ok(());
            }
            self.skip_field(ty)?;
        }
    }

    /// Advances past fields until `tag` is under the cursor.
    ///
    /// Returns the field type when the tag matches, or `None` when a field
    /// with a higher tag (or the struct end) appears first — the cursor stays
    /// on that field either way.
    fn skip_to_tag(&mut self, tag: u8) -> Result<Option<u8>, JceError> {
        while self.remaining() > 0 {
            let (next_tag, ty) = self.peek_head()?;
            if ty == TYPE_STRUCT_END || next_tag >= tag {
                return Ok((next_tag == tag && ty != TYPE_STRUCT_END).then_some(ty));
            }
            self.read_head()?;
            self.skip_field(ty)?;
        }
        Ok(None)
    }

    pub fn read_i64(&mut self, tag: u8) -> Result<Option<i64>, JceError> {
        let Some(ty) = self.skip_to_tag(tag)? else {
            return Ok(None);
        };
        self.read_head()?;
        Ok(Some(self.read_i64_value(ty)?))
    }

    #[allow(dead_code)] // used by tests and kept for protocol completeness
    pub fn read_bool(&mut self, tag: u8) -> Result<Option<bool>, JceError> {
        Ok(self.read_i64(tag)?.map(|value| value != 0))
    }

    pub fn read_string(&mut self, tag: u8) -> Result<Option<String>, JceError> {
        let Some(ty) = self.skip_to_tag(tag)? else {
            return Ok(None);
        };
        self.read_head()?;
        let len = match ty {
            TYPE_STRING1 => u8::from_be_bytes(self.take(1)?.try_into().unwrap()) as usize,
            TYPE_STRING4 => u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as usize,
            _ => return Err(JceError::TypeMismatch { tag, ty }),
        };
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec())
            .map(Some)
            .map_err(|_| JceError::InvalidUtf8)
    }

    pub fn read_bytes(&mut self, tag: u8) -> Result<Option<Vec<u8>>, JceError> {
        let Some(ty) = self.skip_to_tag(tag)? else {
            return Ok(None);
        };
        if ty != TYPE_SIMPLELIST {
            return Err(JceError::TypeMismatch { tag, ty });
        }
        self.read_head()?;
        // The blob length is prefixed by its own type descriptor head (tag 0,
        // type int8), then the length value itself.
        let (_, head_ty) = self.read_head()?;
        if head_ty != TYPE_INT8 {
            return Err(JceError::TypeMismatch {
                tag: 0,
                ty: head_ty,
            });
        }
        let len = self.read_i64_value_at_tag(0)? as usize;
        let bytes = self.take(len)?.to_vec();
        Ok(Some(bytes))
    }

    /// Enters a nested struct if present, returning whether it was found.
    pub fn enter_struct(&mut self, tag: u8) -> Result<bool, JceError> {
        let Some(ty) = self.skip_to_tag(tag)? else {
            return Ok(false);
        };
        if ty != TYPE_STRUCT_BEGIN {
            return Err(JceError::TypeMismatch { tag, ty });
        }
        self.read_head()?;
        Ok(true)
    }

    /// Consumes the current struct's `STRUCTEND` marker.
    pub fn leave_struct(&mut self) -> Result<(), JceError> {
        self.skip_to_struct_end()
    }

    /// Enters a map if present, returning its entry count.
    #[allow(dead_code)] // used by tests
    pub fn enter_map(&mut self, tag: u8) -> Result<Option<usize>, JceError> {
        let Some(ty) = self.skip_to_tag(tag)? else {
            return Ok(None);
        };
        if ty != TYPE_MAP {
            return Err(JceError::TypeMismatch { tag, ty });
        }
        self.read_head()?;
        Ok(Some(self.read_i64_value_at_tag(0)? as usize))
    }
}

impl From<JceError> for crate::DanmuStreamError {
    fn from(err: JceError) -> Self {
        crate::DanmuStreamError::MessageParseError {
            err: err.to_string(),
        }
    }
}
