//! Access to browser localStorage. Store owners retain their own keys, formats,
//! size limits and recovery policies; unavailable storage is always an error.

pub(crate) fn local_storage() -> Result<web_sys::Storage, String> {
    web_sys::window()
        .ok_or_else(|| "browser window is unavailable".to_owned())?
        .local_storage()
        .map_err(|error| format!("open browser localStorage: {error:?}"))?
        .ok_or_else(|| "browser localStorage is unavailable".to_owned())
}
