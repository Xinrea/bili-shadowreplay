use core::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StreamType {
    TS,
    FMP4,
}

pub trait Stream: fmt::Display {
    fn stream_type(&self) -> StreamType;
    fn index(&self) -> String;
    fn ts_url(&self, seg_name: &str) -> String;
    fn is_expired(&self) -> bool;
}
