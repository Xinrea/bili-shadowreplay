#[cfg(not(feature = "headless"))]
#[macro_export]
macro_rules! state_type {
    () => {
        TauriState<'_, State>
    };
}

#[cfg(feature = "headless")]
#[macro_export]
macro_rules! state_type {
    () => {
        State
    };
}
