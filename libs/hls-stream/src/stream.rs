mod bilibili;
mod douyin;

use crate::errors::StreamError;
use crate::message::StreamMessage;
use std::sync::mpsc::Receiver;

pub trait Stream {
    /// 启动流并返回一个接收器，用于接收 StreamMessage
    fn start(&self) -> Result<Receiver<StreamMessage>, StreamError>;

    /// 停止流
    fn stop(&self) -> Result<(), StreamError>;

    /// 重置流状态
    fn reset(&self) -> Result<(), StreamError>;
}
