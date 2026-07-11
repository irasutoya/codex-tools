use crate::models::{
    AccountAuthKind, AuthAccount, AuthService, ProviderAccount, ProviderProfile, ProviderProtocol,
    SessionSummary,
};
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
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS auth_accounts(
               id TEXT PRIMARY KEY, service TEXT NOT NULL, name TEXT NOT NULL,
               login TEXT, email TEXT, credential_json TEXT, config_snapshot TEXT,
               scopes_json TEXT NOT NULL DEFAULT '[]', expires_at INTEGER,
               active INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_auth_accounts_service ON auth_accounts(service);
             CREATE UNIQUE INDEX IF NOT EXISTS idx_auth_accounts_active_openai
               ON auth_accounts(service) WHERE active=1 AND service='openai';
             CREATE TABLE IF NOT EXISTS unified_sessions(
               identity TEXT PRIMARY KEY, thread_id TEXT NOT NULL, host_id TEXT,
               title TEXT NOT NULL DEFAULT '', cwd TEXT NOT NULL DEFAULT '',
               original_provider TEXT NOT NULL DEFAULT '', effective_provider TEXT NOT NULL DEFAULT '',
               archived INTEGER NOT NULL DEFAULT 0, has_user_event INTEGER NOT NULL DEFAULT 0,
               source_rollout TEXT, source_db TEXT NOT NULL DEFAULT '',
               created_at INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL DEFAULT 0,
               last_indexed_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_unified_sessions_thread ON unified_sessions(thread_id);
             CREATE INDEX IF NOT EXISTS idx_unified_sessions_updated ON unified_sessions(updated_at DESC);
             CREATE TABLE IF NOT EXISTS session_provider_origins(
               codex_home TEXT NOT NULL, thread_id TEXT NOT NULL,
               original_provider TEXT NOT NULL, captured_at INTEGER NOT NULL,
               PRIMARY KEY(codex_home,thread_id)
             );",
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

    pub fn auth_accounts(&self) -> anyhow::Result<Vec<AuthAccount>> {
        let db = self.connect()?;
        let mut statement = db.prepare(
            "SELECT id,service,name,login,email,credential_json,config_snapshot,scopes_json,
                    expires_at,active,created_at,updated_at
             FROM auth_accounts ORDER BY active DESC,service,name",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(AuthAccount {
                id: row.get(0)?,
                service: if row.get::<_, String>(1)? == "github" {
                    AuthService::GitHub
                } else {
                    AuthService::OpenAi
                },
                name: row.get(2)?,
                login: row.get(3)?,
                email: row.get(4)?,
                credential: row
                    .get::<_, Option<String>>(5)?
                    .and_then(|value| serde_json::from_str(&value).ok()),
                config_snapshot: row.get(6)?,
                scopes: json_or_default(row.get(7)?),
                expires_at: row.get(8)?,
                active: row.get::<_, i64>(9)? != 0,
                created_at: row.get(10)?,
                updated_at: row.get(11)?,
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
        self.connect()?.execute(
            "INSERT INTO auth_accounts(id,service,name,login,email,credential_json,config_snapshot,
             scopes_json,expires_at,active,created_at,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,COALESCE(NULLIF(?11,0),strftime('%s','now')),strftime('%s','now'))
             ON CONFLICT(id) DO UPDATE SET name=excluded.name,login=excluded.login,email=excluded.email,
             credential_json=excluded.credential_json,config_snapshot=excluded.config_snapshot,
             scopes_json=excluded.scopes_json,expires_at=excluded.expires_at,updated_at=excluded.updated_at",
            params![
                account.id,
                if account.service == AuthService::GitHub { "github" } else { "openai" },
                account.name,
                account.login,
                account.email,
                account.credential.as_ref().map(serde_json::to_string).transpose()?,
                account.config_snapshot,
                serde_json::to_string(&account.scopes)?,
                account.expires_at,
                account.active,
                account.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn activate_auth_account(&self, id: &str) -> anyhow::Result<()> {
        let mut db = self.connect()?;
        let tx = db.transaction()?;
        tx.execute(
            "UPDATE auth_accounts SET active=0 WHERE service='openai'",
            [],
        )?;
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

    pub fn replace_unified_sessions(&self, sessions: &[SessionSummary]) -> anyhow::Result<()> {
        let mut db = self.connect()?;
        let tx = db.transaction()?;
        tx.execute("DELETE FROM unified_sessions", [])?;
        let now = chrono::Utc::now().timestamp();
        for session in sessions {
            tx.execute(
                "INSERT INTO unified_sessions(identity,thread_id,title,cwd,original_provider,
                 effective_provider,archived,has_user_event,source_rollout,source_db,updated_at,last_indexed_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![
                    session.identity,
                    session.id,
                    session.title,
                    session.cwd,
                    session.original_provider,
                    session.provider,
                    session.archived,
                    session.has_user_event,
                    session.source_rollout,
                    session.source_db,
                    session.updated_at,
                    now,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn remember_session_origins(
        &self,
        codex_home: &Path,
        thread_ids: &[String],
        provider: &str,
    ) -> anyhow::Result<usize> {
        let mut db = self.connect()?;
        let tx = db.transaction()?;
        let mut inserted = 0;
        for id in thread_ids {
            inserted += tx.execute(
                "INSERT OR IGNORE INTO session_provider_origins(
                   codex_home,thread_id,original_provider,captured_at
                 ) VALUES(?1,?2,?3,strftime('%s','now'))",
                params![codex_home.display().to_string(), id, provider],
            )?;
        }
        tx.commit()?;
        Ok(inserted)
    }

    pub fn session_origins(
        &self,
        codex_home: &Path,
        provider: &str,
    ) -> anyhow::Result<Vec<String>> {
        let db = self.connect()?;
        let mut statement = db.prepare(
            "SELECT thread_id FROM session_provider_origins
             WHERE codex_home=?1 AND original_provider=?2 ORDER BY captured_at,thread_id",
        )?;
        let rows = statement
            .query_map(params![codex_home.display().to_string(), provider], |row| {
                row.get(0)
            })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn forget_session_origins(
        &self,
        codex_home: &Path,
        provider: &str,
    ) -> anyhow::Result<usize> {
        Ok(self.connect()?.execute(
            "DELETE FROM session_provider_origins WHERE codex_home=?1 AND original_provider=?2",
            params![codex_home.display().to_string(), provider],
        )?)
    }

    pub fn unified_sessions(&self, query: Option<&str>) -> anyhow::Result<Vec<SessionSummary>> {
        let needle = format!("%{}%", query.unwrap_or_default().to_lowercase());
        let db = self.connect()?;
        let mut statement = db.prepare(
            "SELECT identity,thread_id,title,effective_provider,cwd,archived,updated_at,source_db,
                    source_rollout,original_provider,has_user_event
             FROM unified_sessions
             WHERE ?1='%%' OR lower(thread_id || ' ' || title || ' ' || effective_provider || ' ' || cwd) LIKE ?1
             ORDER BY updated_at DESC LIMIT 1000",
        )?;
        let rows = statement.query_map([needle], |row| {
            Ok(SessionSummary {
                identity: row.get(0)?,
                id: row.get(1)?,
                title: row.get(2)?,
                provider: row.get(3)?,
                cwd: row.get(4)?,
                archived: row.get::<_, i64>(5)? != 0,
                updated_at: row.get(6)?,
                source_db: row.get(7)?,
                source_rollout: row.get(8)?,
                original_provider: row.get(9)?,
                has_user_event: row.get::<_, i64>(10)? != 0,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
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

    #[test]
    fn stores_auth_accounts_and_rebuildable_session_index() {
        let temp = tempdir().unwrap();
        let store = Store {
            path: temp.path().join("store.db"),
        };
        let db = store.connect().unwrap();
        db.execute_batch(
            "CREATE TABLE auth_accounts(
               id TEXT PRIMARY KEY,service TEXT,name TEXT,login TEXT,email TEXT,
               credential_json TEXT,config_snapshot TEXT,scopes_json TEXT,expires_at INTEGER,
               active INTEGER,created_at INTEGER,updated_at INTEGER
             );
             CREATE TABLE unified_sessions(
               identity TEXT PRIMARY KEY,thread_id TEXT,host_id TEXT,title TEXT,cwd TEXT,
               original_provider TEXT,effective_provider TEXT,archived INTEGER,has_user_event INTEGER,
               source_rollout TEXT,source_db TEXT,created_at INTEGER,updated_at INTEGER,last_indexed_at INTEGER
             );",
        )
        .unwrap();
        drop(db);
        let account = AuthAccount {
            id: "openai-1".into(),
            service: AuthService::OpenAi,
            name: "Official".into(),
            login: None,
            email: Some("user@example.test".into()),
            credential: Some(serde_json::json!({"tokens":{"access_token":"secret"}})),
            config_snapshot: Some("model = \"gpt\"".into()),
            scopes: vec![],
            expires_at: None,
            active: false,
            created_at: 1,
            updated_at: 1,
        };
        store.save_auth_account(&account).unwrap();
        assert_eq!(store.auth_accounts().unwrap()[0].email, account.email);

        let session = SessionSummary {
            identity: "rollout:file".into(),
            id: "thread-1".into(),
            title: "Conversation".into(),
            provider: "custom".into(),
            cwd: "C:/work".into(),
            archived: false,
            updated_at: 2,
            source_db: "state.db".into(),
            source_rollout: Some("rollout.jsonl".into()),
            original_provider: "openai".into(),
            has_user_event: true,
        };
        store.replace_unified_sessions(&[session]).unwrap();
        let indexed = store.unified_sessions(Some("conversation")).unwrap();
        assert_eq!(indexed.len(), 1);
        assert_eq!(indexed[0].original_provider, "openai");
        assert!(indexed[0].has_user_event);
    }
}
