use crate::errors::StreamError;
use crate::message::StreamMessage;
use crate::stream::Stream;
use std::sync::mpsc::Receiver;

pub struct DouyinStream {}

impl Stream for DouyinStream {
    fn start(&self) -> Result<Receiver<StreamMessage>, StreamError> {
        todo!()
    }

    fn stop(&self) -> Result<(), StreamError> {
        todo!()
    }

    fn reset(&self) -> Result<(), StreamError> {
        todo!()
    }
}
