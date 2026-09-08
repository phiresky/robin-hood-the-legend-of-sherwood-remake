//! Durable native game-identity seed storage.
//!
//! The leaderboard signer and the optional iroh multiplayer transport both
//! derive their Ed25519 keys from this one seed. Keeping persistence here,
//! outside the multiplayer feature, prevents feature selection from creating
//! a second player identity.

use fs2::FileExt;
use std::io::Write;
use std::path::{Path, PathBuf};

const IDENTITY_KEY_FILE: &str = "multiplayer_identity.key";

/// Load the per-install Ed25519 seed, creating it on first use.
pub(crate) fn durable_game_identity_seed() -> Result<[u8; 32], String> {
    load_or_create_seed_at(&identity_key_path()?)
}

fn identity_key_path() -> Result<PathBuf, String> {
    if let Ok(dir) = std::env::var("ROBINHOOD_SAVE_DIR") {
        return Ok(PathBuf::from(dir).join(IDENTITY_KEY_FILE));
    }
    #[cfg(feature = "native-fs")]
    if let Some(data_dir) = dirs::data_dir() {
        return Ok(data_dir.join("robin_hood").join(IDENTITY_KEY_FILE));
    }
    Err(format!(
        "no data directory available to store the game identity key `{IDENTITY_KEY_FILE}`"
    ))
}

fn load_or_create_seed_at(path: &Path) -> Result<[u8; 32], String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("game identity path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("create {}: {error}", parent.display()))?;

    // Native startup paths can request the identity concurrently (for example,
    // leaderboard initialization and multiplayer hosting). Serialize first-use
    // creation so both receive the exact same durable seed.
    let lock_path = parent.join(format!("{IDENTITY_KEY_FILE}.lock"));
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|error| format!("open game identity lock {}: {error}", lock_path.display()))?;
    lock.lock_exclusive()
        .map_err(|error| format!("lock game identity {}: {error}", lock_path.display()))?;

    match read_seed(path) {
        Ok(seed) => Ok(seed),
        Err(ReadSeedError::NotFound) => {
            // Identity creation is outside simulation and requires fresh cryptographic entropy.
            #[allow(clippy::disallowed_methods)]
            let seed = rand::random::<[u8; 32]>();
            write_new_seed(path, &seed)?;
            tracing::info!(path = %path.display(), "generated new durable game identity key");
            Ok(seed)
        }
        Err(ReadSeedError::Invalid(message)) => Err(message),
    }
}

enum ReadSeedError {
    NotFound,
    Invalid(String),
}

fn read_seed(path: &Path) -> Result<[u8; 32], ReadSeedError> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(ReadSeedError::NotFound);
        }
        Err(error) => {
            return Err(ReadSeedError::Invalid(format!(
                "read game identity key {}: {error}",
                path.display()
            )));
        }
    };
    let encoded = contents.trim();
    if encoded.len() != 64 {
        return Err(ReadSeedError::Invalid(format!(
            "corrupt game identity key {}: expected 64 hexadecimal characters, found {}",
            path.display(),
            encoded.len()
        )));
    }
    let mut seed = [0; 32];
    hex::decode_to_slice(encoded, &mut seed).map_err(|error| {
        ReadSeedError::Invalid(format!(
            "corrupt game identity key {}: {error}",
            path.display()
        ))
    })?;
    Ok(seed)
}

fn write_new_seed(path: &Path, seed: &[u8; 32]) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("create game identity key {}: {error}", path.display()))?;
    let encoded = hex::encode(seed);
    if let Err(error) = file
        .write_all(encoded.as_bytes())
        .and_then(|()| file.sync_all())
    {
        // This process created the path while holding the identity lock. A
        // partial private key is unusable and must not shadow a later retry.
        let _ = std::fs::remove_file(path);
        return Err(format!(
            "write game identity key {}: {error}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_created_once_and_reloaded_exactly() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(IDENTITY_KEY_FILE);

        let created = load_or_create_seed_at(&path).unwrap();
        let reloaded = load_or_create_seed_at(&path).unwrap();

        assert_eq!(reloaded, created);
        assert_eq!(std::fs::read_to_string(path).unwrap(), hex::encode(created));
        assert_ne!(created, [0; 32]);
    }

    #[test]
    fn malformed_seed_is_an_error_instead_of_a_new_identity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(IDENTITY_KEY_FILE);
        std::fs::write(&path, "00").unwrap();

        let error = load_or_create_seed_at(&path).unwrap_err();

        assert!(error.contains("expected 64 hexadecimal characters"));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "00");
    }
}
