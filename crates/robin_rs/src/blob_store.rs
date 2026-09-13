//! The one platform persistence shim for small keyed blobs.
//!
//! Natively a key is a file path and writes are atomic publications through
//! [`crate::desktop_persistence`]. In the browser a key is a localStorage key.
//! Store owners retain their own keys, formats, size limits and recovery
//! policies; a missing entry is `Ok(None)`, while unavailable storage is always
//! an error, never an empty store.

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub(crate) enum BlobStoreError {
    /// A native filesystem read or atomic publication failed. Publication
    /// errors keep their [`crate::desktop_persistence::PublicationFailure`].
    #[cfg(not(target_arch = "wasm32"))]
    #[error(transparent)]
    Io(std::io::Error),
    /// The browser window or its localStorage cannot be opened.
    #[cfg(target_arch = "wasm32")]
    #[error("{0}")]
    Unavailable(String),
    /// localStorage rejected an operation; the detail is the JS exception.
    #[cfg(target_arch = "wasm32")]
    #[error("{0}")]
    Rejected(String),
}

impl BlobStoreError {
    /// Convert for `std::io` stores. Native errors pass through unchanged;
    /// browser operation failures are labelled `<operation>: <detail>` while
    /// store-unavailability messages stay bare.
    pub(crate) fn into_io(self, operation: &str) -> std::io::Error {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Io(error) => {
                let _ = operation;
                error
            }
            #[cfg(target_arch = "wasm32")]
            Self::Unavailable(message) => std::io::Error::other(message),
            #[cfg(target_arch = "wasm32")]
            Self::Rejected(detail) => std::io::Error::other(format!("{operation}: {detail}")),
        }
    }
}

/// Text blobs: every store sharing this shim persists UTF-8 JSON (browser
/// localStorage can hold nothing else).
// TODO: add byte-oriented methods if native binary stores (autosave payloads,
// thumbnails) ever move onto this trait.
pub(crate) trait BlobStore {
    /// Native file path or browser localStorage key.
    type Key: ?Sized;

    /// `Ok(None)` only when the entry does not exist.
    fn read_text(&self, key: &Self::Key) -> Result<Option<String>, BlobStoreError>;
    /// Replace the entry atomically.
    fn write_text(&self, key: &Self::Key, text: &str) -> Result<(), BlobStoreError>;
}

/// Files published with the default user-archive staging and durability
/// contract of [`crate::desktop_persistence::write_bytes`].
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct NativeFileStore;

#[cfg(not(target_arch = "wasm32"))]
impl BlobStore for NativeFileStore {
    type Key = Path;

    /// Non-UTF-8 content is an `InvalidData` error, not a missing entry.
    fn read_text(&self, key: &Path) -> Result<Option<String>, BlobStoreError> {
        match std::fs::read_to_string(key) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(BlobStoreError::Io(error)),
        }
    }

    fn write_text(&self, key: &Path, text: &str) -> Result<(), BlobStoreError> {
        crate::desktop_persistence::write_bytes(key, text.as_bytes()).map_err(BlobStoreError::Io)
    }
}

/// An opened browser localStorage. Each localStorage key commits atomically.
#[cfg(target_arch = "wasm32")]
#[derive(Debug, Clone)]
pub(crate) struct BrowserLocalStorage(web_sys::Storage);

#[cfg(target_arch = "wasm32")]
impl BrowserLocalStorage {
    pub(crate) fn open() -> Result<Self, BlobStoreError> {
        web_sys::window()
            .ok_or_else(|| BlobStoreError::Unavailable("browser window is unavailable".to_owned()))?
            .local_storage()
            .map_err(|error| {
                BlobStoreError::Unavailable(format!("open browser localStorage: {error:?}"))
            })?
            .ok_or_else(|| {
                BlobStoreError::Unavailable("browser localStorage is unavailable".to_owned())
            })
            .map(Self)
    }

    /// Snapshot of every key currently stored, in localStorage index order.
    pub(crate) fn keys(&self) -> Result<Vec<String>, BlobStoreError> {
        let length = self.0.length().map_err(rejected)?;
        let mut keys = Vec::with_capacity(length as usize);
        for index in 0..length {
            if let Some(key) = self.0.key(index).map_err(rejected)? {
                keys.push(key);
            }
        }
        Ok(keys)
    }

    /// Removing an absent key succeeds.
    pub(crate) fn remove(&self, key: &str) -> Result<(), BlobStoreError> {
        self.0.remove_item(key).map_err(rejected)
    }
}

#[cfg(target_arch = "wasm32")]
fn rejected(error: wasm_bindgen::JsValue) -> BlobStoreError {
    BlobStoreError::Rejected(format!("{error:?}"))
}

#[cfg(target_arch = "wasm32")]
impl BlobStore for BrowserLocalStorage {
    type Key = str;

    fn read_text(&self, key: &str) -> Result<Option<String>, BlobStoreError> {
        self.0.get_item(key).map_err(rejected)
    }

    fn write_text(&self, key: &str, text: &str) -> Result<(), BlobStoreError> {
        self.0.set_item(key, text).map_err(rejected)
    }
}

/// The platform's store: native files, or the browser's localStorage.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) type PlatformBlobStore = NativeFileStore;
#[cfg(target_arch = "wasm32")]
pub(crate) type PlatformBlobStore = BrowserLocalStorage;

/// Open the platform store. Only the browser store can be unavailable.
pub(crate) fn open_platform_store() -> Result<PlatformBlobStore, BlobStoreError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Ok(NativeFileStore)
    }
    #[cfg(target_arch = "wasm32")]
    {
        BrowserLocalStorage::open()
    }
}
