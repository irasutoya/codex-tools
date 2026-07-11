use crate::models::{
    AccountAuthKind, AuthAccount, AuthService, ProviderAccount, ProviderProfile, ProviderProtocol,
    RouteSettings,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::PathBuf;

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
            path: root.join("codex-tools.sqlite"),
        };
        store.initialize()?;
        Ok(store)
    }

    fn initialize(&self) -> anyhow::Result<()> {
        self.connect()?.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS providers(
               id TEXT PRIMARY KEY,name TEXT NOT NULL,protocol TEXT NOT NULL,base_url TEXT NOT NULL,
               models_json TEXT NOT NULL DEFAULT '[]',headers_json TEXT NOT NULL DEFAULT '{}',
               timeout_secs INTEGER NOT NULL DEFAULT 30,active INTEGER NOT NULL DEFAULT 0,
               context_window INTEGER,auto_compact_threshold INTEGER,enabled INTEGER NOT NULL DEFAULT 1,
               codex_chat_reasoning_json TEXT,model_metadata_json TEXT NOT NULL DEFAULT '[]',
               updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS provider_accounts(
               id TEXT PRIMARY KEY,provider_id TEXT NOT NULL,name TEXT NOT NULL,api_key TEXT NOT NULL,
               headers_json TEXT NOT NULL DEFAULT '{}',active INTEGER NOT NULL DEFAULT 0,
               created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,
               FOREIGN KEY(provider_id) REFERENCES providers(id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_provider_accounts_provider ON provider_accounts(provider_id);
             CREATE UNIQUE INDEX IF NOT EXISTS idx_provider_accounts_one_active
               ON provider_accounts(provider_id) WHERE active=1;
             CREATE TABLE IF NOT EXISTS auth_accounts(
               id TEXT PRIMARY KEY,service TEXT NOT NULL,name TEXT NOT NULL,login TEXT,email TEXT,
               credential_json TEXT,scopes_json TEXT NOT NULL DEFAULT '[]',expires_at INTEGER,
               active INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_auth_accounts_service ON auth_accounts(service);
             CREATE UNIQUE INDEX IF NOT EXISTS idx_auth_accounts_active_openai
               ON auth_accounts(service) WHERE active=1 AND service='openai';",
        )?;
        Ok(())
    }

    pub fn connect(&self) -> anyhow::Result<Connection> {
        let db = Connection::open(&self.path)?;
        db.execute_batch("PRAGMA foreign_keys=ON;")?;
        Ok(db)
    }

    pub fn providers(&self) -> anyhow::Result<Vec<ProviderProfile>> {
        let db = self.connect()?;
        let mut statement = db.prepare(
            "SELECT p.id,p.name,p.protocol,p.base_url,p.models_json,p.headers_json,
                    p.timeout_secs,p.context_window,p.auto_compact_threshold,p.enabled,p.active,
                    p.codex_chat_reasoning_json,p.model_metadata_json,
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
                models: json_or_default(row.get(4)?),
                headers: json_or_default(row.get(5)?),
                timeout_secs: row.get::<_, i64>(6)? as u64,
                context_window: row.get::<_, Option<i64>>(7)?.map(|value| value as u64),
                auto_compact_threshold: row.get::<_, Option<i64>>(8)?.map(|value| value as u64),
                enabled: row.get::<_, i64>(9)? != 0,
                active: row.get::<_, i64>(10)? != 0,
                codex_chat_reasoning: row
                    .get::<_, Option<String>>(11)?
                    .and_then(|value| serde_json::from_str(&value).ok()),
                model_metadata: json_or_default(row.get(12)?),
                active_account_id: row.get(13)?,
                account_count: row.get::<_, i64>(14)? as u64,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn save_provider(&self, provider: &ProviderProfile) -> anyhow::Result<()> {
        self.connect()?.execute(
            "INSERT INTO providers(
               id,name,protocol,base_url,models_json,headers_json,timeout_secs,active,
               context_window,auto_compact_threshold,enabled,codex_chat_reasoning_json,
               model_metadata_json,updated_at
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,strftime('%s','now'))
             ON CONFLICT(id) DO UPDATE SET name=excluded.name,protocol=excluded.protocol,
               base_url=excluded.base_url,models_json=excluded.models_json,
               headers_json=excluded.headers_json,timeout_secs=excluded.timeout_secs,
               context_window=excluded.context_window,
               auto_compact_threshold=excluded.auto_compact_threshold,enabled=excluded.enabled,
               codex_chat_reasoning_json=excluded.codex_chat_reasoning_json,
               model_metadata_json=excluded.model_metadata_json,updated_at=excluded.updated_at",
            params![
                provider.id,
                provider.name,
                protocol_to_db(provider.protocol),
                provider.base_url,
                serde_json::to_string(&provider.models)?,
                serde_json::to_string(&provider.headers)?,
                provider.timeout_secs,
                provider.active,
                provider.context_window,
                provider.auto_compact_threshold,
                provider.enabled,
                provider
                    .codex_chat_reasoning
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?,
                serde_json::to_string(&provider.model_metadata)?,
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
            "SELECT id,provider_id,name,api_key,headers_json,active,created_at,updated_at
             FROM provider_accounts WHERE provider_id=?1 ORDER BY active DESC,name"
        } else {
            "SELECT id,provider_id,name,api_key,headers_json,active,created_at,updated_at
             FROM provider_accounts ORDER BY active DESC,name"
        };
        let mut statement = db.prepare(sql)?;
        let map = |row: &rusqlite::Row<'_>| {
            Ok(ProviderAccount {
                id: row.get(0)?,
                provider_id: row.get(1)?,
                name: row.get(2)?,
                auth_kind: AccountAuthKind::ApiKey,
                api_key: row.get(3)?,
                auth_json: None,
                headers: json_or_default(row.get(4)?),
                active: row.get::<_, i64>(5)? != 0,
                email: None,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
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
            "INSERT INTO provider_accounts(
               id,provider_id,name,api_key,headers_json,active,created_at,updated_at
             ) VALUES(?1,?2,?3,?4,?5,?6,COALESCE(NULLIF(?7,0),strftime('%s','now')),strftime('%s','now'))
             ON CONFLICT(id) DO UPDATE SET provider_id=excluded.provider_id,name=excluded.name,
               api_key=excluded.api_key,headers_json=excluded.headers_json,updated_at=excluded.updated_at",
            params![
                account.id,
                account.provider_id,
                account.name,
                account.api_key,
                serde_json::to_string(&account.headers)?,
                account.active,
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
        tx.execute("UPDATE auth_accounts SET active=0", [])?;
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

    pub fn active_state(&self) -> anyhow::Result<(Option<String>, Option<String>, Option<String>)> {
        let db = self.connect()?;
        let provider = active_id(&db, "providers")?;
        let account = active_id(&db, "provider_accounts")?;
        let official = active_id(&db, "auth_accounts")?;
        Ok((provider, account, official))
    }

    pub fn restore_active(
        &self,
        state: (Option<&str>, Option<&str>, Option<&str>),
    ) -> anyhow::Result<()> {
        let mut db = self.connect()?;
        let tx = db.transaction()?;
        tx.execute("UPDATE providers SET active=0", [])?;
        tx.execute("UPDATE provider_accounts SET active=0", [])?;
        tx.execute("UPDATE auth_accounts SET active=0", [])?;
        for (table, id) in [
            ("providers", state.0),
            ("provider_accounts", state.1),
            ("auth_accounts", state.2),
        ] {
            if let Some(id) = id {
                tx.execute(&format!("UPDATE {table} SET active=1 WHERE id=?1"), [id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn route_settings(&self) -> anyhow::Result<RouteSettings> {
        let value = self
            .connect()?
            .query_row(
                "SELECT value FROM settings WHERE key='local_route'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(value
            .as_deref()
            .and_then(|value| serde_json::from_str(value).ok())
            .unwrap_or_default())
    }

    pub fn save_route_settings(&self, settings: &RouteSettings) -> anyhow::Result<()> {
        self.connect()?.execute(
            "INSERT INTO settings(key,value) VALUES('local_route',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [serde_json::to_string(settings)?],
        )?;
        Ok(())
    }

    pub fn auth_accounts(&self) -> anyhow::Result<Vec<AuthAccount>> {
        let db = self.connect()?;
        let mut statement = db.prepare(
            "SELECT id,name,login,email,credential_json,scopes_json,expires_at,active,created_at,updated_at
             FROM auth_accounts WHERE service='openai' ORDER BY active DESC,name",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(AuthAccount {
                id: row.get(0)?,
                service: AuthService::OpenAi,
                name: row.get(1)?,
                login: row.get(2)?,
                email: row.get(3)?,
                credential: row
                    .get::<_, Option<String>>(4)?
                    .and_then(|value| serde_json::from_str(&value).ok()),
                scopes: json_or_default(row.get(5)?),
                expires_at: row.get(6)?,
                active: row.get::<_, i64>(7)? != 0,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn auth_account(&self, id: &str) -> anyhow::Result<AuthAccount> {
        self.auth_accounts()?
            .into_iter()
            .find(|account| account.id == id)
            .ok_or_else(|| anyhow::anyhow!("认证账号不存在"))
    }

    pub fn save_auth_account(&self, account: &AuthAccount) -> anyhow::Result<()> {
        let mut db = self.connect()?;
        let tx = db.transaction()?;
        let existing: Option<(String, bool, i64)> = account
            .login
            .as_deref()
            .filter(|login| !login.trim().is_empty())
            .map(|login| {
                tx.query_row(
                    "SELECT id,active,created_at FROM auth_accounts
                     WHERE service='openai' AND login=?1 ORDER BY active DESC,updated_at DESC LIMIT 1",
                    [login],
                    |row| Ok((row.get(0)?, row.get::<_, i64>(1)? != 0, row.get(2)?)),
                )
                .optional()
            })
            .transpose()?
            .flatten();
        let id = existing
            .as_ref()
            .map(|value| value.0.as_str())
            .unwrap_or(&account.id);
        let active = existing
            .as_ref()
            .map(|value| value.1)
            .unwrap_or(account.active);
        let created_at = existing
            .as_ref()
            .map(|value| value.2)
            .unwrap_or(account.created_at);
        tx.execute(
            "INSERT INTO auth_accounts(
               id,service,name,login,email,credential_json,scopes_json,expires_at,active,created_at,updated_at
             ) VALUES(?1,'openai',?2,?3,?4,?5,?6,?7,?8,
               COALESCE(NULLIF(?9,0),strftime('%s','now')),strftime('%s','now'))
             ON CONFLICT(id) DO UPDATE SET name=excluded.name,login=excluded.login,
               email=excluded.email,credential_json=excluded.credential_json,
               scopes_json=excluded.scopes_json,expires_at=excluded.expires_at,
               updated_at=excluded.updated_at",
            params![
                id,
                account.name,
                account.login,
                account.email,
                account
                    .credential
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?,
                serde_json::to_string(&account.scopes)?,
                account.expires_at,
                active,
                created_at,
            ],
        )?;
        if let Some(login) = account.login.as_deref() {
            tx.execute(
                "DELETE FROM auth_accounts WHERE service='openai' AND login=?1 AND id<>?2",
                params![login, id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn activate_auth_account(&self, id: &str) -> anyhow::Result<()> {
        let mut db = self.connect()?;
        let tx = db.transaction()?;
        tx.execute("UPDATE providers SET active=0", [])?;
        tx.execute("UPDATE provider_accounts SET active=0", [])?;
        tx.execute("UPDATE auth_accounts SET active=0", [])?;
        let changed = tx.execute(
            "UPDATE auth_accounts SET active=1 WHERE id=?1 AND service='openai'",
            [id],
        )?;
        if changed != 1 {
            anyhow::bail!("只能激活 OpenAI 官方登录账号");
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_auth_account(&self, id: &str) -> anyhow::Result<()> {
        let changed = self
            .connect()?
            .execute("DELETE FROM auth_accounts WHERE id=?1 AND active=0", [id])?;
        if changed != 1 {
            anyhow::bail!("当前认证账号不能删除");
        }
        Ok(())
    }
}

fn active_id(db: &Connection, table: &str) -> anyhow::Result<Option<String>> {
    Ok(db
        .query_row(
            &format!("SELECT id FROM {table} WHERE active=1 LIMIT 1"),
            [],
            |row| row.get(0),
        )
        .optional()?)
}

fn json_or_default<T: serde::de::DeserializeOwned + Default>(value: String) -> T {
    serde_json::from_str(&value).unwrap_or_default()
}

fn protocol_to_db(protocol: ProviderProtocol) -> &'static str {
    match protocol {
        ProviderProtocol::Responses => "responses",
        ProviderProtocol::ChatCompletions => "chat_completions",
    }
}

fn protocol_from_db(value: &str) -> ProviderProtocol {
    match value {
        "chat_completions" => ProviderProtocol::ChatCompletions,
        _ => ProviderProtocol::Responses,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store() -> (tempfile::TempDir, Store) {
        let temp = tempdir().unwrap();
        let store = Store {
            path: temp.path().join("codex-tools.sqlite"),
        };
        store.initialize().unwrap();
        (temp, store)
    }

    #[test]
    fn creates_only_current_application_schema() {
        let (_temp, store) = store();
        let db = store.connect().unwrap();
        let tables = db
            .prepare(
                "SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(
            tables,
            vec![
                "auth_accounts",
                "provider_accounts",
                "providers",
                "settings"
            ]
        );
        let provider_columns = db
            .prepare("PRAGMA table_info(providers)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(provider_columns.len(), 14);
        assert_eq!(provider_columns.first().map(String::as_str), Some("id"));
        assert_eq!(
            provider_columns.last().map(String::as_str),
            Some("updated_at")
        );
    }

    #[test]
    fn route_settings_round_trip() {
        let (_temp, store) = store();
        let settings = RouteSettings {
            enabled: false,
            listen_address: "127.0.0.1".into(),
            port: 8123,
        };
        store.save_route_settings(&settings).unwrap();
        let saved = store.route_settings().unwrap();
        assert!(!saved.enabled);
        assert_eq!(saved.listen_address, "127.0.0.1");
        assert_eq!(saved.port, 8123);
    }
}
