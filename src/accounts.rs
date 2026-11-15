//! Accounts: who you are in Hearth, kept on this computer.
//!
//! Each account has a stable id that never changes, so friends on the home network see the same
//! person every time Hearth starts (before accounts, every start looked like a new person).
//! Accounts live in `%APPDATA%\Hearth\accounts\<id>.json`; `accounts.json` remembers who to sign
//! in automatically. A PIN is optional and only stored as a salted SHA-256 hash.

use crate::model::Data;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct AccountSummary {
    pub id: u64,
    pub username: String,
    pub name: String,
    pub hue: u8,
    pub has_pin: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct Index {
    /// Sign this account in without asking.
    #[serde(default)]
    remember: Option<u64>,
}

pub struct Store {
    root: PathBuf,
}

impl Store {
    /// The Hearth folder in the user's app data.
    pub fn open() -> Option<Store> {
        let base = std::env::var_os("APPDATA").or_else(|| std::env::var_os("HOME"))?;
        Some(Store { root: PathBuf::from(base).join("Hearth") })
    }

    #[cfg(test)]
    pub fn at(root: PathBuf) -> Store {
        Store { root }
    }

    fn dir(&self) -> PathBuf {
        self.root.join("accounts")
    }

    pub fn path_for(&self, id: u64) -> PathBuf {
        self.dir().join(format!("{id:016x}.json"))
    }

    fn read(&self, path: &std::path::Path) -> Option<Data> {
        let bytes = std::fs::read(path).ok()?;
        serde_json::from_slice::<Data>(&bytes).ok()
    }

    /// Every account on this computer, sorted by name.
    pub fn list(&self) -> Vec<AccountSummary> {
        let Ok(entries) = std::fs::read_dir(self.dir()) else { return vec![] };
        let mut out: Vec<AccountSummary> = entries
            .flatten()
            .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
            .filter_map(|e| self.read(&e.path()))
            .filter(|d| d.account.id != 0)
            .map(|d| AccountSummary {
                id: d.account.id,
                username: d.account.username.clone(),
                name: d.me().name.clone(),
                hue: d.me().hue,
                has_pin: d.account.pin_hash.is_some(),
            })
            .collect();
        out.sort_by_key(|a| a.name.to_lowercase());
        out
    }

    pub fn load(&self, id: u64) -> Option<Data> {
        self.read(&self.path_for(id)).filter(|d| d.account.id == id)
    }

    /// Data saved by versions before accounts existed (offered when making the first account).
    pub fn legacy(&self) -> Option<Data> {
        self.read(&self.root.join("state.json"))
    }

    pub fn remembered(&self) -> Option<u64> {
        let bytes = std::fs::read(self.root.join("accounts.json")).ok()?;
        serde_json::from_slice::<Index>(&bytes).ok()?.remember
    }

    pub fn set_remembered(&self, id: Option<u64>) {
        let _ = std::fs::create_dir_all(&self.root);
        if let Ok(bytes) = serde_json::to_vec(&Index { remember: id }) {
            let _ = std::fs::write(self.root.join("accounts.json"), bytes);
        }
    }

    pub fn save(&self, data: &Data) -> Result<PathBuf, String> {
        let path = self.path_for(data.account.id);
        std::fs::create_dir_all(self.dir()).map_err(|e| format!("Couldn't save the account: {e}"))?;
        let bytes = serde_json::to_vec(data).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, bytes).map_err(|e| format!("Couldn't save the account: {e}"))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("Couldn't save the account: {e}"))?;
        Ok(path)
    }
}

/// 3–20 letters, digits, dots, dashes or underscores.
pub fn check_username(name: &str, taken: &[AccountSummary]) -> Result<String, String> {
    let u = name.trim().trim_start_matches('@').to_lowercase();
    if u.chars().count() < 3 || u.chars().count() > 20 {
        return Err("Usernames are 3 to 20 characters.".into());
    }
    if !u.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-') {
        return Err("Use only letters, numbers, dots, dashes and underscores.".into());
    }
    if taken.iter().any(|a| a.username == u) {
        return Err(format!("@{u} is already an account on this computer."));
    }
    Ok(u)
}

/// A PIN is 4–8 digits.
pub fn check_pin(pin: &str) -> Result<(), String> {
    if pin.len() < 4 || pin.len() > 8 || !pin.chars().all(|c| c.is_ascii_digit()) {
        return Err("A PIN is 4 to 8 digits.".into());
    }
    Ok(())
}

pub fn hash_pin(pin: &str, salt: u64) -> String {
    let mut h = Sha256::new();
    h.update(salt.to_le_bytes());
    h.update(pin.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

pub fn pin_matches(data: &Data, pin: &str) -> bool {
    match &data.account.pin_hash {
        None => true,
        Some(h) => *h == hash_pin(pin, data.account.pin_salt),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{sample, Account};

    fn temp_store(tag: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("hearth-accounts-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Store::at(dir)
    }

    #[test]
    fn an_account_keeps_its_id_and_data_between_starts() {
        let store = temp_store("keep");
        let mut d = sample(0, "Lidar");
        d.account = Account { id: 0xABCD, username: "lidar".into(), created: 0, pin_hash: None, pin_salt: 0 };
        store.save(&d).unwrap();
        store.set_remembered(Some(0xABCD));
        // "Restart": everything comes back the same.
        assert_eq!(store.remembered(), Some(0xABCD));
        let back = store.load(0xABCD).unwrap();
        assert_eq!(back.account.id, 0xABCD);
        assert_eq!(back.me().name, "Lidar");
        let list = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].username, "lidar");
        let _ = std::fs::remove_dir_all(&store.root);
    }

    #[test]
    fn usernames_and_pins_are_checked() {
        let taken = vec![AccountSummary { id: 1, username: "mira".into(), name: "Mira".into(), hue: 0, has_pin: false }];
        assert_eq!(check_username(" @Lidar_1 ", &taken).unwrap(), "lidar_1");
        assert!(check_username("mira", &taken).is_err());
        assert!(check_username("ab", &taken).is_err());
        assert!(check_username("no spaces", &taken).is_err());
        assert!(check_pin("1234").is_ok());
        assert!(check_pin("12a4").is_err());
        assert!(check_pin("123").is_err());
    }

    #[test]
    fn a_pin_is_stored_only_as_a_hash() {
        let mut d = sample(0, "P");
        d.account.pin_salt = 99;
        d.account.pin_hash = Some(hash_pin("2468", 99));
        assert!(pin_matches(&d, "2468"));
        assert!(!pin_matches(&d, "1357"));
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("2468"));
    }
}
