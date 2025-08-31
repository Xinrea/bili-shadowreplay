// This file is copied from https://github.com/eatradish/felgens/blob/master/src/pack.rs

use std::io::Read;

use flate2::read::ZlibDecoder;
use scroll::Pread;
use scroll_derive::Pread;

use crate::DanmuStreamError;

#[derive(Debug, Pread, Clone)]
struct BilibiliPackHeader {
    pack_len: u32,
    _header_len: u16,
    ver: u16,
    _op: u32,
    _seq: u32,
}

#[derive(Debug, Pread)]
struct PackHotCount {
    count: u32,
}

type BilibiliPackCtx<'a> = (BilibiliPackHeader, &'a [u8]);

fn pack(buffer: &[u8]) -> Result<BilibiliPackCtx<'_>, DanmuStreamError> {
    let data = buffer
        .pread_with(0, scroll::BE)
        .map_err(|e: scroll::Error| DanmuStreamError::PackError { err: e.to_string() })?;

    let buf = &buffer[16..];

    Ok((data, buf))
}

fn write_int(buffer: &[u8], start: usize, val: u32) -> Vec<u8> {
    let val_bytes = val.to_be_bytes();

    let mut buf = buffer.to_vec();

    for (i, c) in val_bytes.iter().enumerate() {
        buf[start + i] = *c;
    }

    buf
}

pub fn encode(s: &str, op: u8) -> Vec<u8> {
    let data = s.as_bytes();
    let packet_len = 16 + data.len();
    let header = vec![0, 0, 0, 0, 0, 16, 0, 1, 0, 0, 0, op, 0, 0, 0, 1];

    let header = write_int(&header, 0, packet_len as u32);

    [&header, data].concat()
}

pub fn build_pack(buf: &[u8]) -> Result<Vec<String>, DanmuStreamError> {
    let ctx = pack(buf)?;
    let msgs = decode(ctx)?;

    Ok(msgs)
}

fn get_hot_count(body: &[u8]) -> Result<u32, DanmuStreamError> {
    let count = body
        .pread_with::<PackHotCount>(0, scroll::BE)
        .map_err(|e| DanmuStreamError::PackError { err: e.to_string() })?
        .count;

    Ok(count)
}

fn zlib_decode(body: &[u8]) -> Result<(BilibiliPackHeader, Vec<u8>), DanmuStreamError> {
    let mut buf = vec![];
    let mut z = ZlibDecoder::new(body);
    z.read_to_end(&mut buf)
        .map_err(|e| DanmuStreamError::PackError { err: e.to_string() })?;

    let ctx = pack(&buf)?;
    let header = ctx.0;
    let buf = ctx.1.to_vec();

    Ok((header, buf))
}

fn decode(ctx: BilibiliPackCtx) -> Result<Vec<String>, DanmuStreamError> {
    let (mut header, body) = ctx;

    let mut buf = body.to_vec();

    loop {
        (header, buf) = match header.ver {
            2 => zlib_decode(&buf)?,
            3 => brotli_decode(&buf)?,
            0 | 1 => break,
            _ => break,
        }
    }

    let msgs = match header.ver {
        0 => split_msgs(buf, header)?,
        1 => vec![format!("{{\"count\": {}}}", get_hot_count(&buf)?)],
        x => return Err(DanmuStreamError::UnsupportProto { proto: x }),
    };

    Ok(msgs)
}

fn split_msgs(buf: Vec<u8>, header: BilibiliPackHeader) -> Result<Vec<String>, DanmuStreamError> {
    let mut buf = buf;
    let mut header = header;
    let mut msgs = vec![];
    let mut offset = 0;
    let buf_len = buf.len();

    msgs.push(
        std::str::from_utf8(&buf[..(header.pack_len - 16) as usize])
            .map_err(|e| DanmuStreamError::PackError { err: e.to_string() })?
            .to_string(),
    );
    buf = buf[(header.pack_len - 16) as usize..].to_vec();
    offset += header.pack_len - 16;

    while offset != buf_len as u32 {
        let ctx = pack(&buf).map_err(|e| DanmuStreamError::PackError { err: e.to_string() })?;

        header = ctx.0;
        buf = ctx.1.to_vec();

        msgs.push(
            std::str::from_utf8(&buf[..(header.pack_len - 16) as usize])
                .map_err(|e| DanmuStreamError::PackError { err: e.to_string() })?
                .to_string(),
        );

        buf = buf[(header.pack_len - 16) as usize..].to_vec();

        offset += header.pack_len;
    }

    Ok(msgs)
}

fn brotli_decode(body: &[u8]) -> Result<(BilibiliPackHeader, Vec<u8>), DanmuStreamError> {
    let mut reader = brotli::Decompressor::new(body, 4096);

    let mut buf = Vec::new();

    reader
        .read_to_end(&mut buf)
        .map_err(|e| DanmuStreamError::PackError { err: e.to_string() })?;

    let ctx = pack(&buf).map_err(|e| DanmuStreamError::PackError { err: e.to_string() })?;

    let header = ctx.0;
    let buf = ctx.1.to_vec();

    Ok((header, buf))
}
