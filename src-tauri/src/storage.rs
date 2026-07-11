use crate::models::{AccountAuthKind, ProviderAccount, ProviderProfile, ProviderProtocol};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn open() -> anyhow::Result<Self> {
        let root = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("CodexTools");
        std::fs::create_dir_all(&root)?;
        let store = Self {
            path: root.join("codex-tools.db"),
        };
        let db = store.connect()?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS providers(
               id TEXT PRIMARY KEY,name TEXT NOT NULL,protocol TEXT NOT NULL,
               base_url TEXT NOT NULL,api_key TEXT NOT NULL DEFAULT '',default_model TEXT NOT NULL,
               models_json TEXT NOT NULL DEFAULT '[]',headers_json TEXT NOT NULL DEFAULT '{}',
               timeout_secs INTEGER NOT NULL DEFAULT 30,active INTEGER NOT NULL DEFAULT 0,
               updated_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS operations(id TEXT PRIMARY KEY,kind TEXT NOT NULL,payload TEXT NOT NULL,created_at INTEGER NOT NULL,consumed INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS backups(id TEXT PRIMARY KEY,kind TEXT NOT NULL,path TEXT NOT NULL,created_at INTEGER NOT NULL,pinned INTEGER NOT NULL DEFAULT 0);",
        )?;
        add_column(&db, "providers", "context_window", "INTEGER")?;
        add_column(&db, "providers", "auto_compact_threshold", "INTEGER")?;
        add_column(&db, "providers", "enabled", "INTEGER NOT NULL DEFAULT 1")?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS provider_accounts(
               id TEXT PRIMARY KEY,
               provider_id TEXT,
               name TEXT NOT NULL,
               auth_kind TEXT NOT NULL,
               api_key TEXT,
               auth_json TEXT,
               headers_json TEXT NOT NULL DEFAULT '{}',
               active INTEGER NOT NULL DEFAULT 0,
               email TEXT,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL,
               FOREIGN KEY(provider_id) REFERENCES providers(id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_provider_accounts_provider ON provider_accounts(provider_id);
             CREATE UNIQUE INDEX IF NOT EXISTS idx_provider_accounts_one_active ON provider_accounts(provider_id) WHERE active=1;",
        )?;
        migrate_legacy_keys(&db)?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn connect(&self) -> anyhow::Result<Connection> {
        let db = Connection::open(&self.path)?;
        db.execute_batch("PRAGMA foreign_keys=ON;")?;
        Ok(db)
    }

    pub fn providers(&self) -> anyhow::Result<Vec<ProviderProfile>> {
        let db = self.connect()?;
        let mut statement = db.prepare(
            "SELECT p.id,p.name,p.protocol,p.base_url,p.default_model,p.models_json,
                    p.headers_json,p.timeout_secs,p.context_window,p.auto_compact_threshold,
                    p.enabled,p.active,
                    (SELECT id FROM provider_accounts a WHERE a.provider_id=p.id AND a.active=1 LIMIT 1),
                    (SELECT count(*) FROM provider_accounts a WHERE a.provider_id=p.id)
             FROM providers p ORDER BY p.active DESC,p.name",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(ProviderProfile {
                id: row.get(0)?,
                name: row.get(1)?,
                protocol: protocol_from_db(&row.get::<_, String>(2)?),
                base_url: row.get(3)?,
                default_model: row.get(4)?,
                models: json_or_default(row.get(5)?),
                headers: json_or_default(row.get(6)?),
                timeout_secs: row.get::<_, i64>(7)? as u64,
                context_window: row.get::<_, Option<i64>>(8)?.map(|value| value as u64),
                auto_compact_threshold: row.get::<_, Option<i64>>(9)?.map(|value| value as u64),
                enabled: row.get::<_, i64>(10)? != 0,
                active: row.get::<_, i64>(11)? != 0,
                active_account_id: row.get(12)?,
                account_count: row.get::<_, i64>(13)? as u64,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn save_provider(&self, provider: &ProviderProfile) -> anyhow::Result<()> {
        self.connect()?.execute(
            "INSERT INTO providers(id,name,protocol,base_url,api_key,default_model,models_json,headers_json,timeout_secs,active,updated_at,context_window,auto_compact_threshold,enabled)
             VALUES(?1,?2,?3,?4,'',?5,?6,?7,?8,?9,strftime('%s','now'),?10,?11,?12)
             ON CONFLICT(id) DO UPDATE SET name=excluded.name,protocol=excluded.protocol,
             base_url=excluded.base_url,default_model=excluded.default_model,models_json=excluded.models_json,
             headers_json=excluded.headers_json,timeout_secs=excluded.timeout_secs,
             context_window=excluded.context_window,auto_compact_threshold=excluded.auto_compact_threshold,
             enabled=excluded.enabled,updated_at=excluded.updated_at",
            params![
                provider.id,
                provider.name,
                protocol_to_db(provider.protocol),
                provider.base_url,
                provider.default_model,
                serde_json::to_string(&provider.models)?,
                serde_json::to_string(&provider.headers)?,
                provider.timeout_secs,
                provider.active,
                provider.context_window,
                provider.auto_compact_threshold,
                provider.enabled,
            ],
        )?;
        Ok(())
    }

    pub fn delete_provider(&self, id: &str) -> anyhow::Result<()> {
        let changed = self
            .connect()?
            .execute("DELETE FROM providers WHERE id=?1 AND active=0", [id])?;
        if changed != 1 {
            anyhow::bail!("当前 Provider 不能删除");
        }
        Ok(())
    }

    pub fn provider(&self, id: &str) -> anyhow::Result<ProviderProfile> {
        self.providers()?
            .into_iter()
            .find(|provider| provider.id == id)
            .ok_or_else(|| anyhow::anyhow!("Provider 不存在"))
    }

    pub fn accounts(&self, provider_id: Option<&str>) -> anyhow::Result<Vec<ProviderAccount>> {
        let db = self.connect()?;
        let sql = if provider_id.is_some() {
            "SELECT id,provider_id,name,auth_kind,api_key,auth_json,headers_json,active,email,created_at,updated_at FROM provider_accounts WHERE provider_id=?1 ORDER BY active DESC,name"
        } else {
            "SELECT id,provider_id,name,auth_kind,api_key,auth_json,headers_json,active,email,created_at,updated_at FROM provider_accounts ORDER BY active DESC,name"
        };
        let mut statement = db.prepare(sql)?;
        let map = |row: &rusqlite::Row<'_>| {
            let auth_json = row
                .get::<_, Option<String>>(5)?
                .and_then(|value| serde_json::from_str(&value).ok());
            Ok(ProviderAccount {
                id: row.get(0)?,
                provider_id: row.get(1)?,
                name: row.get(2)?,
                auth_kind: if row.get::<_, String>(3)? == "official_oauth" {
                    AccountAuthKind::OfficialOauth
                } else {
                    AccountAuthKind::ApiKey
                },
                api_key: row.get(4)?,
                auth_json,
                headers: json_or_default(row.get(6)?),
                active: row.get::<_, i64>(7)? != 0,
                email: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        };
        let rows = match provider_id {
            Some(id) => statement
                .query_map([id], map)?
                .collect::<rusqlite::Result<_>>()?,
            None => statement
                .query_map([], map)?
                .collect::<rusqlite::Result<_>>()?,
        };
        Ok(rows)
    }

    pub fn account(&self, id: &str) -> anyhow::Result<ProviderAccount> {
        self.accounts(None)?
            .into_iter()
            .find(|account| account.id == id)
            .ok_or_else(|| anyhow::anyhow!("账号不存在"))
    }

    pub fn save_account(&self, account: &ProviderAccount) -> anyhow::Result<()> {
        self.connect()?.execute(
            "INSERT INTO provider_accounts(id,provider_id,name,auth_kind,api_key,auth_json,headers_json,active,email,created_at,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,COALESCE(NULLIF(?10,0),strftime('%s','now')),strftime('%s','now'))
             ON CONFLICT(id) DO UPDATE SET provider_id=excluded.provider_id,name=excluded.name,
             auth_kind=excluded.auth_kind,api_key=excluded.api_key,auth_json=excluded.auth_json,
             headers_json=excluded.headers_json,email=excluded.email,updated_at=excluded.updated_at",
            params![
                account.id,
                account.provider_id,
                account.name,
                if account.auth_kind == AccountAuthKind::OfficialOauth { "official_oauth" } else { "api_key" },
                account.api_key,
                account.auth_json.as_ref().map(serde_json::to_string).transpose()?,
                serde_json::to_string(&account.headers)?,
                account.active,
                account.email,
                account.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn delete_account(&self, id: &str) -> anyhow::Result<()> {
        let changed = self.connect()?.execute(
            "DELETE FROM provider_accounts WHERE id=?1 AND active=0",
            [id],
        )?;
        if changed != 1 {
            anyhow::bail!("当前账号不能删除");
        }
        Ok(())
    }

    pub fn activate(&self, provider_id: &str, account_id: &str) -> anyhow::Result<()> {
        let mut db = self.connect()?;
        let tx = db.transaction()?;
        tx.execute("UPDATE providers SET active=0", [])?;
        tx.execute("UPDATE provider_accounts SET active=0", [])?;
        tx.execute("UPDATE providers SET active=1 WHERE id=?1", [provider_id])?;
        let changed = tx.execute(
            "UPDATE provider_accounts SET active=1 WHERE id=?1 AND provider_id=?2",
            params![account_id, provider_id],
        )?;
        if changed != 1 {
            anyhow::bail!("账号不属于所选 Provider");
        }
        tx.commit()?;
        Ok(())
    }

    pub fn activate_official(&self, account_id: &str) -> anyhow::Result<()> {
        let mut db = self.connect()?;
        let tx = db.transaction()?;
        tx.execute("UPDATE providers SET active=0", [])?;
        tx.execute("UPDATE provider_accounts SET active=0", [])?;
        tx.execute(
            "UPDATE provider_accounts SET active=1 WHERE id=?1 AND auth_kind='official_oauth'",
            [account_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn active_state(&self) -> anyhow::Result<(Option<String>, Option<String>)> {
        let db = self.connect()?;
        let provider = db
            .query_row(
                "SELECT id FROM providers WHERE active=1 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let account = db
            .query_row(
                "SELECT id FROM provider_accounts WHERE active=1 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok((provider, account))
    }

    pub fn restore_active(&self, state: (Option<&str>, Option<&str>)) -> anyhow::Result<()> {
        let mut db = self.connect()?;
        let tx = db.transaction()?;
        tx.execute("UPDATE providers SET active=0", [])?;
        tx.execute("UPDATE provider_accounts SET active=0", [])?;
        if let Some(id) = state.0 {
            tx.execute("UPDATE providers SET active=1 WHERE id=?1", [id])?;
        }
        if let Some(id) = state.1 {
            tx.execute("UPDATE provider_accounts SET active=1 WHERE id=?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }
}

fn add_column(db: &Connection, table: &str, column: &str, definition: &str) -> anyhow::Result<()> {
    let mut statement = db.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !columns.iter().any(|value| value == column) {
        db.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition}"
        ))?;
    }
    Ok(())
}

fn migrate_legacy_keys(db: &Connection) -> anyhow::Result<()> {
    db.execute(
        "INSERT INTO provider_accounts(id,provider_id,name,auth_kind,api_key,auth_json,headers_json,active,email,created_at,updated_at)
         SELECT 'legacy-' || id,id,'默认账号','api_key',api_key,NULL,'{}',active,NULL,updated_at,updated_at
         FROM providers p WHERE trim(api_key)<>'' AND NOT EXISTS(SELECT 1 FROM provider_accounts a WHERE a.provider_id=p.id)",
        [],
    )?;
    db.execute("UPDATE providers SET api_key='' WHERE api_key<>''", [])?;
    Ok(())
}

fn json_or_default<T: serde::de::DeserializeOwned + Default>(value: String) -> T {
    serde_json::from_str(&value).unwrap_or_default()
}

fn protocol_to_db(protocol: ProviderProtocol) -> &'static str {
    if protocol == ProviderProtocol::ChatCompletions {
        "chat_completions"
    } else {
        "responses"
    }
}

fn protocol_from_db(value: &str) -> ProviderProtocol {
    if value == "chat_completions" {
        ProviderProtocol::ChatCompletions
    } else {
        ProviderProtocol::Responses
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn migrates_legacy_provider_key_to_default_account() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("store.db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE providers(id TEXT PRIMARY KEY,name TEXT NOT NULL,protocol TEXT NOT NULL,base_url TEXT NOT NULL,api_key TEXT NOT NULL,default_model TEXT NOT NULL,models_json TEXT NOT NULL DEFAULT '[]',headers_json TEXT NOT NULL DEFAULT '{}',timeout_secs INTEGER NOT NULL DEFAULT 30,active INTEGER NOT NULL DEFAULT 0,updated_at INTEGER NOT NULL); INSERT INTO providers VALUES('p1','Test','responses','https://example.test/v1','secret','model','[]','{}',30,1,1);").unwrap();
        drop(db);
        let store = Store { path };
        let db = store.connect().unwrap();
        add_column(&db, "providers", "context_window", "INTEGER").unwrap();
        add_column(&db, "providers", "auto_compact_threshold", "INTEGER").unwrap();
        add_column(&db, "providers", "enabled", "INTEGER NOT NULL DEFAULT 1").unwrap();
        db.execute_batch("CREATE TABLE provider_accounts(id TEXT PRIMARY KEY,provider_id TEXT,name TEXT,auth_kind TEXT,api_key TEXT,auth_json TEXT,headers_json TEXT,active INTEGER,email TEXT,created_at INTEGER,updated_at INTEGER);").unwrap();
        migrate_legacy_keys(&db).unwrap();
        let key: String = db
            .query_row("SELECT api_key FROM provider_accounts", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(key, "secret");
    }
}
