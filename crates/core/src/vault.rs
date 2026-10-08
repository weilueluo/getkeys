use crate::{model::*, new_id, now};
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection as Sqlite, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    io::{Read, Write},
    path::Path,
    time::Duration,
};
use zeroize::Zeroizing;

pub struct Vault {
    pub(crate) db: Sqlite,
}

impl Vault {
    pub fn open(path: &Path, key: &[u8; 32]) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = Sqlite::open(path)?;
        db.pragma_update(None, "key", format!("x'{}'", hex::encode(key)))?;
        let cipher: String = db
            .pragma_query_value(None, "cipher_version", |r| r.get(0))
            .context("SQLCipher is required; plaintext SQLite is not supported")?;
        ensure!(!cipher.is_empty(), "SQLCipher is unavailable");
        db.pragma_update(None, "temp_store", "MEMORY")?;
        db.pragma_update(None, "foreign_keys", "ON")?;
        db.busy_timeout(Duration::from_secs(5))?;
        let version: i64 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .context("The vault key is wrong or the vault is damaged")?;
        ensure!(
            version <= 1,
            "This vault was created by a newer GetKeys version"
        );
        db.execute_batch("CREATE TABLE IF NOT EXISTS records (kind TEXT NOT NULL, id TEXT NOT NULL, data TEXT NOT NULL, PRIMARY KEY(kind,id)); CREATE TABLE IF NOT EXISTS rotations (id TEXT PRIMARY KEY, scope TEXT NOT NULL, active INTEGER NOT NULL, data TEXT NOT NULL); CREATE UNIQUE INDEX IF NOT EXISTS one_active_rotation ON rotations(scope) WHERE active=1; PRAGMA user_version=1;")?;
        Ok(Self { db })
    }
    pub fn system(path: &Path) -> Result<Self> {
        let account = path.to_string_lossy();
        let entry = keyring::Entry::new("dev.getkeys.vault", &account)
            .context("The OS credential store is unavailable")?;
        let key = match entry.get_secret() {
            Ok(secret) => Zeroizing::new(secret),
            Err(keyring::Error::NoEntry) if !path.exists() => {
                let mut key = Zeroizing::new(vec![0u8; 32]);
                getrandom::fill(&mut key)
                    .map_err(|_| anyhow::anyhow!("OS randomness is unavailable"))?;
                entry
                    .set_secret(&key)
                    .context("Unable to save the vault key in the OS credential store")?;
                key
            }
            Err(error) => bail!("Cannot unlock the vault from the OS credential store: {error}"),
        };
        let key: &[u8; 32] = key
            .as_slice()
            .try_into()
            .context("Invalid vault key in credential store")?;
        Self::open(path, key)
    }
    pub fn put<T: Serialize>(&self, kind: &str, id: &str, value: &T) -> Result<()> {
        let json = Zeroizing::new(serde_json::to_string(value)?);
        self.db.execute("INSERT INTO records(kind,id,data) VALUES(?1,?2,?3) ON CONFLICT(kind,id) DO UPDATE SET data=excluded.data",params![kind,id,json.as_str()])?;
        Ok(())
    }
    pub fn get<T: DeserializeOwned>(&self, kind: &str, id: &str) -> Result<T> {
        let data: Option<String> = self
            .db
            .query_row(
                "SELECT data FROM records WHERE kind=?1 AND id=?2",
                params![kind, id],
                |r| r.get(0),
            )
            .optional()?;
        let data = Zeroizing::new(data.with_context(|| format!("{kind} was not found"))?);
        serde_json::from_str(&data).context("Invalid vault record")
    }
    pub fn list<T: DeserializeOwned>(&self, kind: &str) -> Result<Vec<T>> {
        let mut stmt = self
            .db
            .prepare("SELECT data FROM records WHERE kind=?1 ORDER BY id")?;
        let rows = stmt.query_map([kind], |r| r.get::<_, String>(0))?;
        rows.map(|r| {
            let data = Zeroizing::new(r?);
            Ok(serde_json::from_str(&data)?)
        })
        .collect()
    }
    pub fn remove(&self, kind: &str, id: &str) -> Result<()> {
        self.db.execute(
            "DELETE FROM records WHERE kind=?1 AND id=?2",
            params![kind, id],
        )?;
        Ok(())
    }
    pub fn event(&self, action: &str, subject: &str) -> Result<()> {
        let event = Activity {
            id: new_id()?,
            timestamp: now(),
            action: action.into(),
            subject: subject.into(),
        };
        self.put("activity", &event.id, &event)
    }
    pub fn rotations(&self) -> Result<Vec<Rotation>> {
        let mut stmt = self
            .db
            .prepare("SELECT data FROM rotations ORDER BY rowid DESC")?;
        stmt.query_map([], |r| r.get::<_, String>(0))?
            .map(|r| Ok(serde_json::from_str(&r?)?))
            .collect()
    }
    pub fn rotation(&self, id: &str) -> Result<Rotation> {
        let data: String =
            self.db
                .query_row("SELECT data FROM rotations WHERE id=?1", [id], |r| r.get(0))?;
        Ok(serde_json::from_str(&data)?)
    }
    pub fn save_rotation(&self, r: &Rotation) -> Result<()> {
        let active = !matches!(r.state, RotationState::Complete | RotationState::Aborted);
        self.db.execute("INSERT INTO rotations(id,scope,active,data) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET active=excluded.active,data=excluded.data",params![r.id,format!("{}:{}",r.connection_id,r.resource),active,serde_json::to_string(r)?]).context("Another rotation is already active for this provider resource")?;
        Ok(())
    }
    pub fn transition(
        &self,
        r: &mut Rotation,
        expected: &RotationState,
        next: RotationState,
    ) -> Result<()> {
        let transaction = self.db.unchecked_transaction()?;
        let current = self.rotation(&r.id)?;
        ensure!(
            &current.state == expected,
            "Rotation changed in another process. Refresh before continuing."
        );
        r.state = next;
        self.save_rotation(r)?;
        transaction.commit()?;
        Ok(())
    }
    pub fn export(&self, path: &Path, passphrase: String) -> Result<()> {
        ensure!(
            passphrase.len() >= 12,
            "Use an export passphrase of at least 12 characters"
        );
        let mut stmt = self
            .db
            .prepare("SELECT kind,id,data FROM records WHERE kind!='auth_flow'")?;
        let records: Vec<(String, String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_, _>>()?;
        let payload = Zeroizing::new(serde_json::to_vec(
            &serde_json::json!({"version":1,"records":records,"rotations":self.rotations()?}),
        )?);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .context("Choose a new export file")?;
        let encryptor = age::Encryptor::with_user_passphrase(passphrase.into());
        let mut writer = encryptor.wrap_output(file)?;
        writer.write_all(&payload)?;
        writer.finish()?;
        Ok(())
    }
    pub fn import_backup(&self, path: &Path, passphrase: String) -> Result<()> {
        let count: i64 = self
            .db
            .query_row("SELECT count(*) FROM records", [], |r| r.get(0))?;
        ensure!(
            count == 0,
            "Restore requires an empty vault to avoid overwriting existing credentials"
        );
        let decryptor = age::Decryptor::new(std::fs::File::open(path)?)?;
        let identity = age::scrypt::Identity::new(passphrase.into());
        let mut reader = decryptor.decrypt(std::iter::once(&identity as &dyn age::Identity))?;
        let mut bytes = Zeroizing::new(Vec::new());
        reader
            .by_ref()
            .take(32 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= 32 * 1024 * 1024,
            "Backup exceeds supported size"
        );
        let payload: serde_json::Value = serde_json::from_slice(&bytes)?;
        ensure!(payload["version"] == 1, "Unsupported backup version");
        let records: Vec<(String, String, String)> =
            serde_json::from_value(payload["records"].clone())?;
        let rotations: Vec<Rotation> = serde_json::from_value(payload["rotations"].clone())?;
        let transaction = self.db.unchecked_transaction()?;
        for (kind, id, data) in records {
            ensure!(
                ["connection", "credential", "binding", "activity"].contains(&kind.as_str()),
                "Unsupported backup record"
            );
            let _: serde_json::Value = serde_json::from_str(&data)?;
            self.db.execute(
                "INSERT INTO records(kind,id,data) VALUES(?1,?2,?3)",
                params![kind, id, data],
            )?;
        }
        for r in rotations {
            self.save_rotation(&r)?;
        }
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encrypted_file_rejects_the_wrong_key_and_hides_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        {
            let vault = Vault::open(&path, &[7; 32]).unwrap();
            vault
                .put("test", "one", &"never-write-this-plaintext")
                .unwrap();
        }
        assert!(Vault::open(&path, &[8; 32]).is_err());
        assert!(
            !String::from_utf8_lossy(&std::fs::read(&path).unwrap())
                .contains("never-write-this-plaintext")
        );
        assert_eq!(
            Vault::open(&path, &[7; 32])
                .unwrap()
                .get::<String>("test", "one")
                .unwrap(),
            "never-write-this-plaintext"
        );
    }
    #[test]
    fn backup_roundtrip_and_wrong_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let source = Vault::open(&dir.path().join("a.db"), &[1; 32]).unwrap();
        source
            .put(
                "credential",
                "c",
                &serde_json::json!({"secret":"test-only"}),
            )
            .unwrap();
        let backup = dir.path().join("backup.age");
        source
            .export(&backup, "twelve or more characters".into())
            .unwrap();
        let target = Vault::open(&dir.path().join("b.db"), &[2; 32]).unwrap();
        assert!(target.import_backup(&backup, "wrong".into()).is_err());
        target
            .import_backup(&backup, "twelve or more characters".into())
            .unwrap();
        assert_eq!(
            target.get::<serde_json::Value>("credential", "c").unwrap()["secret"],
            "test-only"
        );
    }
}
