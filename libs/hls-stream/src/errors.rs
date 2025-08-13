use custom_error::custom_error;

custom_error! {pub StreamError
    StreamNotRunning = "Stream is not running",
    StreamAlreadyRunning = "Stream is already running",
    StreamNotStopped = "Stream is not stopped",
    StreamAlreadyStopped = "Stream is already stopped",
    StreamNotReset = "Stream is not reset",
    StreamAlreadyReset = "Stream is already reset",
}
