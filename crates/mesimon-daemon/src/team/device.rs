//! `~/.local/state/mesimon/team/device.toml`: who this machine is.
//!
//! One seed derives both device keys; the relay address, the display name
//! and the bearer credential sit beside it. 0600, like every other file that
//! holds a secret. Losing it is losing the identity: the person signs in
//! again as a new device and gets re-invited.
use anyhow::{Context, Result};
use mesimon_team::crypto::DeviceKeys;
use mesimon_team::hex;
use mesimon_team::relay::RelayEndpoint;
use mesimon_team::wire::Credential;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const DEVICE_SCHEMA: u32 = 1;

#[derive(Clone, Serialize, Deserialize)]
pub struct DeviceFile {
    pub schema_version: u32,
    /// 32 bytes, hex. Everything else about the identity derives from it.
    pub seed: String,
    pub display_name: String,
    pub relay: RelayEndpoint,
    /// Absent between generating the seed and the relay accepting it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<Credential>,
}

impl std::fmt::Debug for DeviceFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceFile")
            .field("display_name", &self.display_name)
            .field("relay", &self.relay)
            .finish_non_exhaustive()
    }
}

impl DeviceFile {
    pub fn fresh(display_name: String, relay: RelayEndpoint) -> Self {
        Self {
            schema_version: DEVICE_SCHEMA,
            seed: hex::encode(DeviceKeys::generate().seed()),
            display_name,
            relay,
            credential: None,
        }
    }

    pub fn keys(&self) -> Option<DeviceKeys> {
        hex::decode::<32>(&self.seed).map(DeviceKeys::from_seed)
    }

    /// `None` when the file does not exist; an error when it exists and
    /// cannot be read, so a corrupt identity is said out loud rather than
    /// silently replaced.
    pub fn load(path: &Path) -> Result<Option<Self>> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
        };
        let file: Self =
            toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
        if file.schema_version > DEVICE_SCHEMA {
            anyhow::bail!("{} was written by a newer mesimon", path.display());
        }
        if file.keys().is_none() {
            anyhow::bail!("{} holds no usable seed", path.display());
        }
        Ok(Some(file))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            crate::paths::own_private_dir(dir)?;
        }
        crate::store::write_atomic(path, &toml::to_string_pretty(self)?, 0o600)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_file_round_trips_and_keeps_its_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("team").join("device.toml");
        assert!(DeviceFile::load(&path).unwrap().is_none());
        let relay = RelayEndpoint::parse("relay.example:9000").unwrap();
        let mut file = DeviceFile::fresh("Amit".into(), relay.clone());
        let id = file.keys().unwrap().id();
        file.save(&path).unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let back = DeviceFile::load(&path).unwrap().unwrap();
        assert_eq!(back.keys().unwrap().id(), id);
        assert_eq!(back.relay, relay);
        assert!(back.credential.is_none());
        file.credential = Some(Credential::generate());
        file.save(&path).unwrap();
        assert_eq!(DeviceFile::load(&path).unwrap().unwrap().credential, file.credential);
        assert!(!format!("{file:?}").contains(&file.seed));
    }

    #[test]
    fn a_newer_or_broken_file_is_an_error_not_a_new_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device.toml");
        std::fs::write(&path, "schema_version = 99\nseed = \"00\"\ndisplay_name = \"x\"\n[relay]\nhost = \"h\"\nport = 1\n").unwrap();
        assert!(DeviceFile::load(&path).is_err());
        std::fs::write(&path, "not toml at all").unwrap();
        assert!(DeviceFile::load(&path).is_err());
    }
}
