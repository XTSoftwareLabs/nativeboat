//! The SQLite database file format (<https://www.sqlite.org/fileformat2.html>).
//!
//! Reading takes a database file and, when there is one, its write-ahead
//! log: the log's committed frames replace the pages they hold. The file's
//! schema table and the b-trees it names are read whole: tables as rows by
//! rowid, indexes as their key records in order.
//!
//! Writing builds a new file from a database's content: every b-tree is
//! packed from sorted content, page 1 holds the schema table, payloads too
//! large for a page continue on overflow pages, and nothing is left free. The
//! file is in rollback-journal mode, for writers that replace a file whole.

use super::SqlValue;
#[allow(unused_imports)]
use crate::prelude::*;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

const MAGIC: &[u8; 16] = b"SQLite format 3\0";
const HEADER: usize = 100;
const TABLE_INTERIOR: u8 = 0x05;
const TABLE_LEAF: u8 = 0x0D;
const INDEX_INTERIOR: u8 = 0x02;
const INDEX_LEAF: u8 = 0x0A;
/// The SQLite version a written file names as its writer's, 3.45.0.
const WRITER_VERSION: u32 = 3_045_000;
/// The deepest b-tree read: deeper ones are corrupt or cyclic.
const MAX_DEPTH: usize = 40;

/// Why a file cannot be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FormatError {
    /// Not an SQLite database file.
    NotADatabase,
    /// A structure the file names is out of bounds or inconsistent.
    Corrupt(&'static str),
    /// A valid file this reader does not read: UTF-16 text.
    Unsupported(&'static str),
}

/// A row of `sqlite_schema`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SchemaEntry {
    /// `table`, `index`, `view` or `trigger`.
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) table: String,
    pub(crate) root: u32,
    /// The statement that created it; `None` for automatic indexes.
    pub(crate) sql: Option<String>,
}

/// The header fields a database keeps across rewrites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Header {
    pub(crate) page_size: u32,
    pub(crate) change_counter: u32,
    pub(crate) schema_cookie: u32,
    pub(crate) user_version: i32,
    pub(crate) application_id: i32,
}

impl Default for Header {
    fn default() -> Self {
        Header {
            page_size: 4096,
            change_counter: 0,
            schema_cookie: 0,
            user_version: 0,
            application_id: 0,
        }
    }
}

/// A database file's pages, with its write-ahead log applied.
pub(crate) struct File {
    pub(crate) header: Header,
    pages: Vec<Vec<u8>>,
    /// Bytes of each page that hold content: the page size less the
    /// reserved space.
    usable: usize,
}

fn u16_at(bytes: &[u8], at: usize) -> usize {
    usize::from(u16::from_be_bytes([bytes[at], bytes[at + 1]]))
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// A varint at `at`: its value and length.
fn varint(bytes: &[u8], at: usize) -> Result<(u64, usize), FormatError> {
    let mut value = 0_u64;
    for index in 0..9 {
        let byte = *bytes
            .get(at + index)
            .ok_or(FormatError::Corrupt("a varint runs past its page"))?;
        if index == 8 {
            return Ok(((value << 8) | u64::from(byte), 9));
        }
        value = (value << 7) | u64::from(byte & 0x7F);
        if byte & 0x80 == 0 {
            return Ok((value, index + 1));
        }
    }
    unreachable!("a varint has at most nine bytes")
}

/// `value` as a varint.
pub(crate) fn put_varint(out: &mut Vec<u8>, value: u64) {
    if value > 0x00FF_FFFF_FFFF_FFFF {
        let mut bytes = [0_u8; 9];
        bytes[8] = value as u8;
        let mut rest = value >> 8;
        for index in (0..8).rev() {
            bytes[index] = (rest as u8 & 0x7F) | 0x80;
            rest >>= 7;
        }
        out.extend_from_slice(&bytes);
        return;
    }
    let mut groups = [0_u8; 9];
    let mut count = 0;
    let mut rest = value;
    loop {
        groups[count] = (rest & 0x7F) as u8;
        count += 1;
        rest >>= 7;
        if rest == 0 {
            break;
        }
    }
    for index in (0..count).rev() {
        let more = if index > 0 { 0x80 } else { 0 };
        out.push(groups[index] | more);
    }
}

fn varint_len(value: u64) -> usize {
    let mut out = Vec::new();
    put_varint(&mut out, value);
    out.len()
}

/// A record's values, from its payload.
pub(crate) fn decode_record(payload: &[u8]) -> Result<Vec<SqlValue>, FormatError> {
    let (header_size, mut at) = varint(payload, 0)?;
    let header_size = header_size as usize;
    if header_size > payload.len() || header_size < at {
        return Err(FormatError::Corrupt(
            "a record header is larger than its record",
        ));
    }
    let mut body = header_size;
    let mut values = Vec::new();
    while at < header_size {
        let (kind, length) = varint(payload, at)?;
        at += length;
        let size = match kind {
            0 | 8 | 9 => 0,
            1 => 1,
            2 => 2,
            3 => 3,
            4 => 4,
            5 => 6,
            6 | 7 => 8,
            10 | 11 => return Err(FormatError::Corrupt("a record has a reserved serial type")),
            n if n % 2 == 0 => ((n - 12) / 2) as usize,
            n => ((n - 13) / 2) as usize,
        };
        let bytes = payload
            .get(body..body + size)
            .ok_or(FormatError::Corrupt("a record value runs past its record"))?;
        body += size;
        values.push(match kind {
            0 => SqlValue::Null,
            1..=6 => {
                let mut value = if bytes[0] & 0x80 != 0 { -1_i64 } else { 0 };
                for byte in bytes {
                    value = (value << 8) | i64::from(*byte);
                }
                SqlValue::Integer(value)
            }
            7 => SqlValue::Real(f64::from_bits(u64::from_be_bytes(
                bytes.try_into().expect("eight bytes"),
            ))),
            8 => SqlValue::Integer(0),
            9 => SqlValue::Integer(1),
            n if n % 2 == 0 => SqlValue::Blob(bytes.to_vec()),
            _ => SqlValue::Text(String::from_utf8_lossy(bytes).into_owned()),
        });
    }
    Ok(values)
}

/// A record's payload.
pub(crate) fn encode_record(values: &[SqlValue]) -> Vec<u8> {
    let mut kinds = Vec::new();
    let mut body = Vec::new();
    for value in values {
        match value {
            SqlValue::Null => kinds.push(0),
            SqlValue::Integer(0) => kinds.push(8),
            SqlValue::Integer(1) => kinds.push(9),
            SqlValue::Integer(integer) => {
                let integer = *integer;
                let (kind, size) = if (-128..=127).contains(&integer) {
                    (1, 1)
                } else if (-32_768..=32_767).contains(&integer) {
                    (2, 2)
                } else if (-8_388_608..=8_388_607).contains(&integer) {
                    (3, 3)
                } else if (-2_147_483_648..=2_147_483_647).contains(&integer) {
                    (4, 4)
                } else if (-140_737_488_355_328..=140_737_488_355_327).contains(&integer) {
                    (5, 6)
                } else {
                    (6, 8)
                };
                kinds.push(kind);
                body.extend_from_slice(&integer.to_be_bytes()[8 - size..]);
            }
            SqlValue::Real(real) => {
                kinds.push(7);
                body.extend_from_slice(&real.to_bits().to_be_bytes());
            }
            SqlValue::Text(text) => {
                kinds.push(text.len() as u64 * 2 + 13);
                body.extend_from_slice(text.as_bytes());
            }
            SqlValue::Blob(blob) => {
                kinds.push(blob.len() as u64 * 2 + 12);
                body.extend_from_slice(blob);
            }
        }
    }
    let types: usize = kinds.iter().map(|kind| varint_len(*kind)).sum();
    // The header size counts its own varint, whose length depends on it.
    let mut header_size = types + 1;
    if varint_len(header_size as u64) > 1 {
        header_size = types + varint_len((types + 2) as u64);
    }
    let mut out = Vec::with_capacity(header_size + body.len());
    put_varint(&mut out, header_size as u64);
    for kind in kinds {
        put_varint(&mut out, kind);
    }
    out.extend_from_slice(&body);
    out
}

/// The most of a payload a cell keeps on its page, as SQLite computes it:
/// the rest continues on overflow pages.
fn local_size(usable: usize, payload: usize, table_leaf: bool) -> usize {
    let max = if table_leaf {
        usable - 35
    } else {
        (usable - 12) * 64 / 255 - 23
    };
    if payload <= max {
        return payload;
    }
    let min = (usable - 12) * 32 / 255 - 23;
    let local = min + (payload - min) % (usable - 4);
    if local <= max { local } else { min }
}

/// The checksum a write-ahead log keeps over `data`, continuing `sums`.
fn wal_checksum(data: &[u8], big_endian: bool, sums: (u32, u32)) -> (u32, u32) {
    let (mut s0, mut s1) = sums;
    for pair in data.chunks_exact(8) {
        let word = |bytes: &[u8]| {
            let bytes = [bytes[0], bytes[1], bytes[2], bytes[3]];
            if big_endian {
                u32::from_be_bytes(bytes)
            } else {
                u32::from_le_bytes(bytes)
            }
        };
        s0 = s0.wrapping_add(word(&pair[0..4])).wrapping_add(s1);
        s1 = s1.wrapping_add(word(&pair[4..8])).wrapping_add(s0);
    }
    (s0, s1)
}

/// The pages a write-ahead log's committed frames replace, by page number,
/// and the database size the last commit gives.
fn wal_pages(wal: &[u8], page_size: usize) -> (Vec<(u32, Vec<u8>)>, Option<u32>) {
    if wal.len() < 32 {
        return (Vec::new(), None);
    }
    let magic = u32_at(wal, 0);
    let big_endian = match magic {
        0x377F_0682 => false,
        0x377F_0683 => true,
        _ => return (Vec::new(), None),
    };
    if u32_at(wal, 8) as usize != page_size {
        return (Vec::new(), None);
    }
    let (salt1, salt2) = (u32_at(wal, 16), u32_at(wal, 20));
    let mut sums = wal_checksum(&wal[..24], big_endian, (0, 0));
    if sums != (u32_at(wal, 24), u32_at(wal, 28)) {
        return (Vec::new(), None);
    }
    let mut committed = Vec::new();
    let mut pending = Vec::new();
    let mut size = None;
    let mut at = 32;
    while at + 24 + page_size <= wal.len() {
        let frame = &wal[at..at + 24];
        if u32_at(frame, 8) != salt1 || u32_at(frame, 12) != salt2 {
            break;
        }
        sums = wal_checksum(&frame[..8], big_endian, sums);
        sums = wal_checksum(&wal[at + 24..at + 24 + page_size], big_endian, sums);
        if sums != (u32_at(frame, 16), u32_at(frame, 20)) {
            break;
        }
        pending.push((u32_at(frame, 0), wal[at + 24..at + 24 + page_size].to_vec()));
        let commit = u32_at(frame, 4);
        if commit != 0 {
            committed.append(&mut pending);
            size = Some(commit);
        }
        at += 24 + page_size;
    }
    (committed, size)
}

impl File {
    /// Reads a database file and, when there is one, its write-ahead log.
    pub(crate) fn read(bytes: &[u8], wal: Option<&[u8]>) -> Result<File, FormatError> {
        if bytes.len() < HEADER || &bytes[..16] != MAGIC {
            return Err(FormatError::NotADatabase);
        }
        let page_size = match u16_at(bytes, 16) {
            1 => 65_536,
            size if size >= 512 && size.is_power_of_two() => size,
            _ => return Err(FormatError::Corrupt("the page size is not a power of two")),
        };
        let reserved = usize::from(bytes[20]);
        if reserved >= page_size - 480 {
            return Err(FormatError::Corrupt("too much reserved space"));
        }
        if u32_at(bytes, 56) > 1 {
            return Err(FormatError::Unsupported("databases with UTF-16 text"));
        }
        let mut pages: Vec<Vec<u8>> = bytes
            .chunks(page_size)
            .map(|chunk| {
                let mut page = chunk.to_vec();
                page.resize(page_size, 0);
                page
            })
            .collect();
        if let Some(wal) = wal {
            let (frames, size) = wal_pages(wal, page_size);
            for (number, page) in frames {
                let index = number as usize - 1;
                if pages.len() <= index {
                    pages.resize(index + 1, vec![0; page_size]);
                }
                pages[index] = page;
            }
            if let Some(size) = size {
                pages.truncate(size as usize);
            }
        }
        let first = &pages[0];
        let header = Header {
            page_size: page_size as u32,
            change_counter: u32_at(first, 24),
            schema_cookie: u32_at(first, 40),
            user_version: u32_at(first, 60) as i32,
            application_id: u32_at(first, 68) as i32,
        };
        Ok(File {
            header,
            pages,
            usable: page_size - reserved,
        })
    }

    fn page(&self, number: u32) -> Result<&[u8], FormatError> {
        number
            .checked_sub(1)
            .and_then(|index| self.pages.get(index as usize))
            .map(Vec::as_slice)
            .ok_or(FormatError::Corrupt("a page number is out of the file"))
    }

    /// A cell's payload, gathering it from overflow pages.
    fn payload(
        &self,
        page: &[u8],
        at: usize,
        size: usize,
        table_leaf: bool,
    ) -> Result<Vec<u8>, FormatError> {
        let local = local_size(self.usable, size, table_leaf);
        let mut payload = page
            .get(at..at + local)
            .ok_or(FormatError::Corrupt("a cell runs past its page"))?
            .to_vec();
        if local < size {
            let mut next = u32_at(page, at + local);
            let mut hops = 0;
            while payload.len() < size {
                hops += 1;
                if next == 0 || hops > self.pages.len() {
                    return Err(FormatError::Corrupt("an overflow chain ends early"));
                }
                let overflow = self.page(next)?;
                let take = (size - payload.len()).min(self.usable - 4);
                payload.extend_from_slice(&overflow[4..4 + take]);
                next = u32_at(overflow, 0);
            }
        }
        Ok(payload)
    }

    /// The cells of a b-tree page, with its type and right-most child.
    fn cells(&self, number: u32) -> Result<(u8, Vec<usize>, u32), FormatError> {
        let page = self.page(number)?;
        let start = if number == 1 { HEADER } else { 0 };
        let kind = page[start];
        let count = u16_at(page, start + 3);
        let interior = matches!(kind, TABLE_INTERIOR | INDEX_INTERIOR);
        if !matches!(
            kind,
            TABLE_INTERIOR | TABLE_LEAF | INDEX_INTERIOR | INDEX_LEAF
        ) {
            return Err(FormatError::Corrupt("a page is not a b-tree page"));
        }
        let header = if interior { 12 } else { 8 };
        let right = if interior { u32_at(page, start + 8) } else { 0 };
        let mut cells = Vec::with_capacity(count);
        for index in 0..count {
            let pointer = start + header + index * 2;
            if pointer + 2 > page.len() {
                return Err(FormatError::Corrupt("a cell pointer runs past its page"));
            }
            cells.push(u16_at(page, pointer));
        }
        Ok((kind, cells, right))
    }

    /// The rows of the table b-tree at `root`, by rowid in order.
    pub(crate) fn table(&self, root: u32) -> Result<Vec<(i64, Vec<SqlValue>)>, FormatError> {
        let mut rows = Vec::new();
        self.table_into(root, &mut rows, 0)?;
        Ok(rows)
    }

    fn table_into(
        &self,
        number: u32,
        rows: &mut Vec<(i64, Vec<SqlValue>)>,
        depth: usize,
    ) -> Result<(), FormatError> {
        if depth > MAX_DEPTH {
            return Err(FormatError::Corrupt("a b-tree is too deep"));
        }
        let (kind, cells, right) = self.cells(number)?;
        let page = self.page(number)?;
        match kind {
            TABLE_LEAF => {
                for at in cells {
                    let (size, length) = varint(page, at)?;
                    let (rowid, rowid_length) = varint(page, at + length)?;
                    let payload =
                        self.payload(page, at + length + rowid_length, size as usize, true)?;
                    rows.push((rowid as i64, decode_record(&payload)?));
                }
            }
            TABLE_INTERIOR => {
                for at in cells {
                    self.table_into(u32_at(page, at), rows, depth + 1)?;
                }
                self.table_into(right, rows, depth + 1)?;
            }
            _ => return Err(FormatError::Corrupt("a table b-tree holds an index page")),
        }
        Ok(())
    }

    /// The keys of the index b-tree at `root`, in order.
    pub(crate) fn index(&self, root: u32) -> Result<Vec<Vec<SqlValue>>, FormatError> {
        let mut keys = Vec::new();
        self.index_into(root, &mut keys, 0)?;
        Ok(keys)
    }

    fn index_into(
        &self,
        number: u32,
        keys: &mut Vec<Vec<SqlValue>>,
        depth: usize,
    ) -> Result<(), FormatError> {
        if depth > MAX_DEPTH {
            return Err(FormatError::Corrupt("a b-tree is too deep"));
        }
        let (kind, cells, right) = self.cells(number)?;
        let page = self.page(number)?;
        let interior = match kind {
            INDEX_LEAF => false,
            INDEX_INTERIOR => true,
            _ => return Err(FormatError::Corrupt("an index b-tree holds a table page")),
        };
        for at in cells {
            let at = if interior {
                self.index_into(u32_at(page, at), keys, depth + 1)?;
                at + 4
            } else {
                at
            };
            let (size, length) = varint(page, at)?;
            let payload = self.payload(page, at + length, size as usize, false)?;
            keys.push(decode_record(&payload)?);
        }
        if interior {
            self.index_into(right, keys, depth + 1)?;
        }
        Ok(())
    }

    /// The rows of `sqlite_schema`.
    pub(crate) fn schema(&self) -> Result<Vec<SchemaEntry>, FormatError> {
        let text = |value: Option<&SqlValue>| match value {
            Some(SqlValue::Text(text)) => Some(text.clone()),
            _ => None,
        };
        self.table(1)?
            .into_iter()
            .map(|(_, row)| {
                let root = match row.get(3) {
                    Some(SqlValue::Integer(root)) => *root as u32,
                    _ => 0,
                };
                Ok(SchemaEntry {
                    kind: text(row.first())
                        .ok_or(FormatError::Corrupt("a schema row has no type"))?,
                    name: text(row.get(1))
                        .ok_or(FormatError::Corrupt("a schema row has no name"))?,
                    table: text(row.get(2)).unwrap_or_default(),
                    root,
                    sql: text(row.get(4)),
                })
            })
            .collect()
    }
}

/// What a schema object holds, to write it.
pub(crate) enum Content {
    /// A table's rows, by rowid in order.
    Table(Vec<(i64, Vec<SqlValue>)>),
    /// An index's keys, in order.
    Index(Vec<Vec<SqlValue>>),
    /// Views and triggers hold nothing.
    None,
}

/// A schema object to write: its schema row (whose root page the writer
/// assigns) and its content.
pub(crate) struct Object {
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) table: String,
    pub(crate) sql: Option<String>,
    pub(crate) content: Content,
}

/// The pages of a file being written.
struct Writer {
    page_size: usize,
    pages: Vec<Vec<u8>>,
}

/// A cell before it is placed: what precedes its payload (a child pointer,
/// sizes, a rowid) and its payload, part of which goes to overflow pages
/// when the cell is placed. Only placed cells take overflow pages.
struct Pending {
    prefix: Vec<u8>,
    payload: Vec<u8>,
    table_leaf: bool,
}

impl Pending {
    /// The bytes the cell takes on its page.
    fn size(&self, usable: usize) -> usize {
        if self.payload.is_empty() {
            return self.prefix.len();
        }
        let local = local_size(usable, self.payload.len(), self.table_leaf);
        self.prefix.len() + local + if local < self.payload.len() { 4 } else { 0 }
    }

    /// The cell with `child` as its left child, for interior pages.
    fn with_child(&self, child: u32) -> Pending {
        let mut prefix = child.to_be_bytes().to_vec();
        prefix.extend_from_slice(&self.prefix);
        Pending {
            prefix,
            payload: self.payload.clone(),
            table_leaf: false,
        }
    }
}

/// A table b-tree's leaf cell for a row.
fn row_cell(rowid: i64, values: &[SqlValue]) -> Pending {
    let payload = encode_record(values);
    let mut prefix = Vec::new();
    put_varint(&mut prefix, payload.len() as u64);
    put_varint(&mut prefix, rowid as u64);
    Pending {
        prefix,
        payload,
        table_leaf: true,
    }
}

/// An index b-tree's cell for a key, without a child.
fn key_cell(key: &[SqlValue]) -> Pending {
    let payload = encode_record(key);
    let mut prefix = Vec::new();
    put_varint(&mut prefix, payload.len() as u64);
    Pending {
        prefix,
        payload,
        table_leaf: false,
    }
}

/// A table b-tree's interior cell: a child and the largest rowid below it.
fn rowid_cell(child: u32, rowid: i64) -> Pending {
    let mut prefix = child.to_be_bytes().to_vec();
    put_varint(&mut prefix, rowid as u64);
    Pending {
        prefix,
        payload: Vec::new(),
        table_leaf: false,
    }
}

impl Writer {
    fn allocate(&mut self) -> u32 {
        self.pages.push(vec![0; self.page_size]);
        self.pages.len() as u32
    }

    /// Room for cells on a page: its bytes after the header.
    fn room(&self, number: u32, interior: bool) -> usize {
        let start = if number == 1 { HEADER } else { 0 };
        self.page_size - start - if interior { 12 } else { 8 }
    }

    /// Whether `cells` fit on page `number`.
    fn fits(&self, number: u32, interior: bool, cells: &[Pending]) -> bool {
        cells
            .iter()
            .map(|cell| cell.size(self.page_size) + 2)
            .sum::<usize>()
            <= self.room(number, interior)
    }

    /// The bytes of a placed cell, its overflow moved to new pages.
    fn place(&mut self, cell: &Pending) -> Vec<u8> {
        let mut bytes = cell.prefix.clone();
        if cell.payload.is_empty() {
            return bytes;
        }
        let usable = self.page_size;
        let local = local_size(usable, cell.payload.len(), cell.table_leaf);
        bytes.extend_from_slice(&cell.payload[..local]);
        if local < cell.payload.len() {
            let chunks: Vec<&[u8]> = cell.payload[local..].chunks(usable - 4).collect();
            let numbers: Vec<u32> = chunks.iter().map(|_| self.allocate()).collect();
            for (index, chunk) in chunks.iter().enumerate() {
                let next = numbers.get(index + 1).copied().unwrap_or(0);
                let page = &mut self.pages[numbers[index] as usize - 1];
                page[..4].copy_from_slice(&next.to_be_bytes());
                page[4..4 + chunk.len()].copy_from_slice(chunk);
            }
            bytes.extend_from_slice(&numbers[0].to_be_bytes());
        }
        bytes
    }

    /// Writes a b-tree page of `cells`, with `right` as the right-most child
    /// of an interior page.
    fn write_page(&mut self, number: u32, kind: u8, cells: &[Pending], right: Option<u32>) {
        let placed: Vec<Vec<u8>> = cells.iter().map(|cell| self.place(cell)).collect();
        let start = if number == 1 { HEADER } else { 0 };
        let header = if right.is_some() { 12 } else { 8 };
        let page_size = self.page_size;
        let page = &mut self.pages[number as usize - 1];
        let mut content = page_size;
        for (index, bytes) in placed.iter().enumerate() {
            content -= bytes.len();
            page[content..content + bytes.len()].copy_from_slice(bytes);
            let at = start + header + index * 2;
            page[at..at + 2].copy_from_slice(&(content as u16).to_be_bytes());
        }
        page[start] = kind;
        page[start + 1..start + 3].copy_from_slice(&0_u16.to_be_bytes());
        page[start + 3..start + 5].copy_from_slice(&(placed.len() as u16).to_be_bytes());
        let area = if content == 65_536 { 0 } else { content as u16 };
        page[start + 5..start + 7].copy_from_slice(&area.to_be_bytes());
        page[start + 7] = 0;
        if let Some(right) = right {
            page[start + 8..start + 12].copy_from_slice(&right.to_be_bytes());
        }
    }

    /// Splits `cells` into runs that each fill a fresh page.
    fn runs(&self, cells: Vec<Pending>, interior: bool) -> Vec<Vec<Pending>> {
        let room = self.page_size - if interior { 12 } else { 8 };
        let mut runs = Vec::new();
        let mut current = Vec::new();
        let mut used = 0;
        for cell in cells {
            let cost = cell.size(self.page_size) + 2;
            if !current.is_empty() && used + cost > room {
                runs.push(core::mem::take(&mut current));
                used = 0;
            }
            used += cost;
            current.push(cell);
        }
        runs.push(current);
        runs
    }

    /// Writes a table b-tree of `rows` with its root at `root`.
    fn table(&mut self, root: u32, rows: &[(i64, Vec<SqlValue>)]) {
        let cells: Vec<Pending> = rows
            .iter()
            .map(|(rowid, values)| row_cell(*rowid, values))
            .collect();
        if self.fits(root, false, &cells) {
            self.write_page(root, TABLE_LEAF, &cells, None);
            return;
        }
        // Leaves on fresh pages, each known by its largest rowid.
        let mut level: Vec<(u32, i64)> = Vec::new();
        let mut taken = 0;
        for run in self.runs(cells, false) {
            taken += run.len();
            let page = self.allocate();
            self.write_page(page, TABLE_LEAF, &run, None);
            level.push((page, rows[taken - 1].0));
        }
        // Interior levels, until one fits the root.
        loop {
            let (right, _) = *level.last().expect("a level has children");
            let cells: Vec<Pending> = level[..level.len() - 1]
                .iter()
                .map(|(child, rowid)| rowid_cell(*child, *rowid))
                .collect();
            if self.fits(root, true, &cells) {
                self.write_page(root, TABLE_INTERIOR, &cells, Some(right));
                return;
            }
            // Pages of children: each page's last child is its right
            // pointer, and the page is known by that child's rowid. A page
            // holds at least two children: one cell and its right pointer.
            let room = self.page_size - 12;
            let mut groups: Vec<Vec<(u32, i64)>> = Vec::new();
            let mut group: Vec<(u32, i64)> = Vec::new();
            let mut used = 0;
            for (child, rowid) in level.iter().copied() {
                let cost = rowid_cell(child, rowid).size(self.page_size) + 2;
                if group.len() > 1 && used + cost > room {
                    groups.push(core::mem::take(&mut group));
                    used = 0;
                }
                used += cost;
                group.push((child, rowid));
            }
            groups.push(group);
            let count = groups.len();
            if count > 1 && groups[count - 1].len() < 2 {
                let moved = groups[count - 2].pop().expect("a full page has children");
                groups[count - 1].insert(0, moved);
            }
            level = groups
                .iter()
                .map(|group| self.interior_table_page(group))
                .collect();
        }
    }

    /// An interior table page over `children`, known by its last child's
    /// rowid.
    fn interior_table_page(&mut self, children: &[(u32, i64)]) -> (u32, i64) {
        let (right, rowid) = *children.last().expect("an interior page has children");
        let cells: Vec<Pending> = children[..children.len() - 1]
            .iter()
            .map(|(child, rowid)| rowid_cell(*child, *rowid))
            .collect();
        let page = self.allocate();
        self.write_page(page, TABLE_INTERIOR, &cells, Some(right));
        (page, rowid)
    }

    /// Writes an index b-tree of `keys` with its root at `root`.
    fn index(&mut self, root: u32, keys: &[Vec<SqlValue>]) {
        let cells: Vec<Pending> = keys.iter().map(|key| key_cell(key)).collect();
        if self.fits(root, false, &cells) {
            self.write_page(root, INDEX_LEAF, &cells, None);
            return;
        }
        // Leaves of runs of keys, each separated from the next by one key
        // that moves up to the level above. No leaf is empty: when the last
        // key would separate an empty leaf, the key before it separates and
        // the last key ends the tree.
        let room = self.page_size - 8;
        let mut runs: Vec<Vec<Pending>> = Vec::new();
        let mut separators = Vec::new();
        let mut current = Vec::new();
        let mut used = 0;
        for cell in cells {
            let cost = cell.size(self.page_size) + 2;
            if !current.is_empty() && used + cost > room {
                runs.push(core::mem::take(&mut current));
                separators.push(cell);
                used = 0;
                continue;
            }
            used += cost;
            current.push(cell);
        }
        runs.push(current);
        let count = runs.len();
        if count > 1 && runs[count - 1].is_empty() {
            let last = separators.pop().expect("a separator before the empty leaf");
            let before = runs[count - 2].pop().expect("a full leaf has keys");
            separators.push(before);
            runs[count - 1].push(last);
        }
        let mut children = Vec::new();
        for run in &runs {
            let page = self.allocate();
            self.write_page(page, INDEX_LEAF, run, None);
            children.push(page);
        }
        // Interior levels: separator `i` lies between children `i` and
        // `i + 1`, and the cell holding it points to child `i`.
        loop {
            let cells: Vec<Pending> = separators
                .iter()
                .zip(&children)
                .map(|(separator, child)| separator.with_child(*child))
                .collect();
            let right = *children.last().expect("an index level has children");
            if self.fits(root, true, &cells) {
                self.write_page(root, INDEX_INTERIOR, &cells, Some(right));
                return;
            }
            // Pages of cells: a page ends with the child before the next
            // separator, which moves up a level. No page is without cells.
            let room = self.page_size - 12;
            let mut pages: Vec<(Vec<usize>, usize)> = Vec::new();
            let mut promoted: Vec<usize> = Vec::new();
            let mut group = Vec::new();
            let mut used = 0;
            for (index, cell) in cells.iter().enumerate() {
                let cost = cell.size(self.page_size) + 2;
                if !group.is_empty() && used + cost > room {
                    pages.push((core::mem::take(&mut group), index));
                    promoted.push(index);
                    used = 0;
                    continue;
                }
                used += cost;
                group.push(index);
            }
            pages.push((group, children.len() - 1));
            let count = pages.len();
            if count > 1 && pages[count - 1].0.is_empty() {
                let promoted_last = promoted.pop().expect("a promoted separator");
                let moved = pages[count - 2].0.pop().expect("a full page has cells");
                pages[count - 2].1 = moved;
                promoted.push(moved);
                pages[count - 1].0.push(promoted_last);
            }
            let mut next_children = Vec::new();
            for (group, right) in &pages {
                let page_cells: Vec<Pending> = group
                    .iter()
                    .map(|index| separators[*index].with_child(children[*index]))
                    .collect();
                let page = self.allocate();
                self.write_page(page, INDEX_INTERIOR, &page_cells, Some(children[*right]));
                next_children.push(page);
            }
            let next_separators = promoted
                .iter()
                .map(|index| Pending {
                    prefix: separators[*index].prefix.clone(),
                    payload: separators[*index].payload.clone(),
                    table_leaf: false,
                })
                .collect();
            children = next_children;
            separators = next_separators;
        }
    }
}

/// A database file holding `objects`, in schema order, with `header`'s
/// fields; the change counter is the header's plus one.
pub(crate) fn write(header: &Header, objects: &[Object]) -> Vec<u8> {
    let page_size = header.page_size as usize;
    let mut writer = Writer {
        page_size,
        pages: vec![vec![0; page_size]],
    };
    // Roots first, so schema rows can name them.
    let roots: Vec<u32> = objects
        .iter()
        .map(|object| match object.content {
            Content::None => 0,
            _ => writer.allocate(),
        })
        .collect();
    for (object, root) in objects.iter().zip(&roots) {
        match &object.content {
            Content::Table(rows) => writer.table(*root, rows),
            Content::Index(keys) => writer.index(*root, keys),
            Content::None => {}
        }
    }
    let schema: Vec<(i64, Vec<SqlValue>)> = objects
        .iter()
        .zip(&roots)
        .enumerate()
        .map(|(index, (object, root))| {
            (
                index as i64 + 1,
                vec![
                    SqlValue::Text(object.kind.clone()),
                    SqlValue::Text(object.name.clone()),
                    SqlValue::Text(object.table.clone()),
                    SqlValue::Integer(i64::from(*root)),
                    object.sql.clone().map_or(SqlValue::Null, SqlValue::Text),
                ],
            )
        })
        .collect();
    writer.table(1, &schema);
    let count = writer.pages.len() as u32;
    let change = header.change_counter.wrapping_add(1);
    let first = &mut writer.pages[0];
    first[..16].copy_from_slice(MAGIC);
    let size_field = if page_size == 65_536 {
        1
    } else {
        page_size as u16
    };
    first[16..18].copy_from_slice(&size_field.to_be_bytes());
    first[18] = 1;
    first[19] = 1;
    first[20] = 0;
    first[21] = 64;
    first[22] = 32;
    first[23] = 32;
    first[24..28].copy_from_slice(&change.to_be_bytes());
    first[28..32].copy_from_slice(&count.to_be_bytes());
    first[32..40].copy_from_slice(&[0; 8]);
    first[40..44].copy_from_slice(&header.schema_cookie.to_be_bytes());
    first[44..48].copy_from_slice(&4_u32.to_be_bytes());
    first[48..56].copy_from_slice(&[0; 8]);
    first[56..60].copy_from_slice(&1_u32.to_be_bytes());
    first[60..64].copy_from_slice(&(header.user_version as u32).to_be_bytes());
    first[64..68].copy_from_slice(&[0; 4]);
    first[68..72].copy_from_slice(&(header.application_id as u32).to_be_bytes());
    first[72..92].copy_from_slice(&[0; 20]);
    first[92..96].copy_from_slice(&change.to_be_bytes());
    first[96..100].copy_from_slice(&WRITER_VERSION.to_be_bytes());
    writer.pages.concat()
}

#[cfg(test)]
mod tests;
