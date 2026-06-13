use rps_core::config::{
    ClientConfig, ControllerConfig, ProxyListenConfig, TunnelConfig, TunnelMode,
};
use rps_core::protocol::TargetProtocol;
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use std::{path::Path, str::FromStr, time::Duration};
use uuid::Uuid;

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

#[derive(Debug, Clone)]
pub struct DbClient {
    pub id: String,
    pub psk: String,
    pub enabled: bool,
    pub remark: Option<String>,
    pub max_connections: Option<u32>,
    pub compress: bool,
    pub encrypt: bool,
}

#[derive(Debug, Clone)]
pub struct DbTunnel {
    pub id: String,
    pub client_id: String,
    pub mode: TunnelMode,
    pub listen: String,
    pub target: Option<String>,
    pub enabled: bool,
    pub expires_at: Option<i64>,
    pub traffic_limit_bytes: Option<u64>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub disabled_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DbProxyAccount {
    pub id: String,
    pub kind: String,
    pub client_id: String,
    pub username: String,
    pub password: String,
    pub enabled: bool,
    pub remark: Option<String>,
    pub expires_at: Option<i64>,
    pub traffic_limit_bytes: Option<u64>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub disabled_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewClient {
    pub id: String,
    pub psk: String,
    pub enabled: bool,
    pub remark: Option<String>,
    pub max_connections: Option<u32>,
    pub compress: bool,
    pub encrypt: bool,
}

#[derive(Debug, Clone)]
pub struct NewTunnel {
    pub id: String,
    pub client_id: String,
    pub mode: TunnelMode,
    pub listen: String,
    pub target: Option<String>,
    pub enabled: bool,
    pub expires_at: Option<i64>,
    pub traffic_limit_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct NewProxyAccount {
    pub id: String,
    pub kind: String,
    pub client_id: String,
    pub username: String,
    pub password: String,
    pub enabled: bool,
    pub remark: Option<String>,
    pub expires_at: Option<i64>,
    pub traffic_limit_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct TrafficCounter {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

impl Database {
    pub async fn open(path: impl AsRef<Path>, config: &ControllerConfig) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            tokio::fs::create_dir_all(parent).await?;
            secure_database_dir(parent).await?;
        }

        let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
        let options = SqliteConnectOptions::from_str(&url)?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;
        let db = Self { pool };
        secure_database_files(path).await?;
        db.migrate().await?;
        db.seed_from_config(config).await?;
        secure_database_files(path).await?;
        Ok(db)
    }

    async fn migrate(&self) -> anyhow::Result<()> {
        for statement in [
            r#"
            create table if not exists clients (
                id text primary key,
                psk text not null unique,
                enabled integer not null default 1,
                remark text,
                max_connections integer,
                compress integer not null default 0,
                encrypt integer not null default 0,
                created_at integer not null,
                updated_at integer not null
            )
            "#,
            r#"
            create table if not exists tunnels (
                id text primary key,
                client_id text not null,
                mode text not null,
                listen text not null,
                target text,
                enabled integer not null default 1,
                expires_at integer,
                traffic_limit_bytes integer,
                rx_bytes integer not null default 0,
                tx_bytes integer not null default 0,
                disabled_reason text,
                created_at integer not null,
                updated_at integer not null
            )
            "#,
            r#"
            create table if not exists proxy_listeners (
                kind text primary key,
                listen text not null,
                client_id text not null,
                enabled integer not null default 1,
                created_at integer not null,
                updated_at integer not null
            )
            "#,
            r#"
            create table if not exists proxy_accounts (
                id text primary key,
                kind text not null,
                client_id text not null,
                username text not null,
                password text not null,
                enabled integer not null default 1,
                remark text,
                expires_at integer,
                traffic_limit_bytes integer,
                rx_bytes integer not null default 0,
                tx_bytes integer not null default 0,
                disabled_reason text,
                created_at integer not null,
                updated_at integer not null,
                unique(kind, username)
            )
            "#,
            r#"
            create table if not exists client_online (
                client_id text primary key,
                online integer not null default 0,
                control_connected_at integer,
                data_connected_at integer,
                last_seen integer
            )
            "#,
            r#"
            create table if not exists agent_sessions (
                id text primary key,
                client_id text not null,
                role text not null,
                remote_addr text,
                connected_at integer not null,
                disconnected_at integer
            )
            "#,
            r#"
            create table if not exists stream_sessions (
                id text primary key,
                client_id text not null,
                tunnel_id text not null,
                protocol text not null,
                target text not null,
                remote_addr text not null,
                rx_bytes integer not null default 0,
                tx_bytes integer not null default 0,
                opened_at integer not null,
                closed_at integer
            )
            "#,
            r#"
            create table if not exists traffic_counters (
                scope text not null,
                key text not null,
                rx_bytes integer not null default 0,
                tx_bytes integer not null default 0,
                updated_at integer not null,
                primary key (scope, key)
            )
            "#,
            r#"
            create table if not exists usage_snapshots (
                id text primary key,
                scope text not null,
                key text not null,
                rx_bytes integer not null default 0,
                tx_bytes integer not null default 0,
                captured_at integer not null
            )
            "#,
        ] {
            sqlx::query(statement).execute(&self.pool).await?;
        }

        self.migrate_clients_psk().await?;
        self.ensure_column("tunnels", "expires_at", "integer")
            .await?;
        self.ensure_column("tunnels", "traffic_limit_bytes", "integer")
            .await?;
        self.ensure_column("tunnels", "rx_bytes", "integer not null default 0")
            .await?;
        self.ensure_column("tunnels", "tx_bytes", "integer not null default 0")
            .await?;
        self.ensure_column("tunnels", "disabled_reason", "text")
            .await?;
        self.ensure_column("proxy_accounts", "expires_at", "integer")
            .await?;
        self.ensure_column("proxy_accounts", "traffic_limit_bytes", "integer")
            .await?;
        self.ensure_column("proxy_accounts", "rx_bytes", "integer not null default 0")
            .await?;
        self.ensure_column("proxy_accounts", "tx_bytes", "integer not null default 0")
            .await?;
        self.ensure_column("proxy_accounts", "disabled_reason", "text")
            .await?;
        Ok(())
    }

    async fn ensure_column(
        &self,
        table: &str,
        column: &str,
        definition: &str,
    ) -> anyhow::Result<()> {
        let rows = sqlx::query(&format!("pragma table_info({table})"))
            .fetch_all(&self.pool)
            .await?;
        let exists = rows.iter().any(|row| {
            row.try_get::<String, _>("name")
                .map(|name| name == column)
                .unwrap_or(false)
        });
        if !exists {
            sqlx::query(&format!(
                "alter table {table} add column {column} {definition}"
            ))
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    async fn migrate_clients_psk(&self) -> anyhow::Result<()> {
        let columns = sqlx::query("pragma table_info(clients)")
            .fetch_all(&self.pool)
            .await?;
        let has_psk = columns.iter().any(|row| {
            row.try_get::<String, _>("name")
                .map(|name| name == "psk")
                .unwrap_or(false)
        });
        let has_vkey = columns.iter().any(|row| {
            row.try_get::<String, _>("name")
                .map(|name| name == "vkey")
                .unwrap_or(false)
        });

        if !has_psk {
            sqlx::query("alter table clients add column psk text")
                .execute(&self.pool)
                .await?;
        }
        if has_vkey {
            for statement in [
                "drop table if exists clients_new",
                "drop table if exists clients_legacy",
                "alter table clients rename to clients_legacy",
                r#"
                create table clients (
                    id text primary key,
                    psk text not null unique,
                    enabled integer not null default 1,
                    remark text,
                    max_connections integer,
                    compress integer not null default 0,
                    encrypt integer not null default 0,
                    created_at integer not null,
                    updated_at integer not null
                )
                "#,
                r#"
                insert or ignore into clients
                    (id, psk, enabled, remark, max_connections, compress, encrypt, created_at, updated_at)
                select
                    id,
                    coalesce(nullif(psk, ''), vkey),
                    enabled,
                    remark,
                    max_connections,
                    compress,
                    encrypt,
                    created_at,
                    updated_at
                from clients_legacy
                where coalesce(nullif(psk, ''), vkey) is not null
                    and coalesce(nullif(psk, ''), vkey) != ''
                "#,
                "drop table clients_legacy",
            ] {
                sqlx::query(statement).execute(&self.pool).await?;
            }
        }
        sqlx::query(
            "create unique index if not exists idx_clients_psk_unique on clients(psk) where psk is not null",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn seed_from_config(&self, config: &ControllerConfig) -> anyhow::Result<()> {
        for client in &config.clients {
            self.insert_config_client(client).await?;
        }
        for tunnel in &config.tunnels {
            self.insert_config_tunnel(tunnel).await?;
        }
        if let Some(proxy) = &config.server.http_proxy {
            self.insert_config_proxy("http", proxy).await?;
        }
        if let Some(proxy) = &config.server.socks5 {
            self.insert_config_proxy("socks5", proxy).await?;
        }
        Ok(())
    }

    async fn insert_config_client(&self, client: &ClientConfig) -> anyhow::Result<()> {
        let now = now_secs();
        sqlx::query(
            r#"
            insert into clients
                (id, psk, enabled, remark, max_connections, compress, encrypt, created_at, updated_at)
            values (?, ?, ?, ?, ?, ?, ?, ?, ?)
            on conflict(id) do update set
                psk = excluded.psk,
                enabled = excluded.enabled,
                remark = excluded.remark,
                max_connections = excluded.max_connections,
                compress = excluded.compress,
                encrypt = excluded.encrypt,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&client.id)
        .bind(&client.psk)
        .bind(bool_to_i64(client.enabled))
        .bind(&client.remark)
        .bind(client.max_connections.map(i64::from))
        .bind(bool_to_i64(client.compress))
        .bind(bool_to_i64(client.encrypt))
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn insert_config_tunnel(&self, tunnel: &TunnelConfig) -> anyhow::Result<()> {
        let now = now_secs();
        sqlx::query(
            r#"
            insert or ignore into tunnels
                (id, client_id, mode, listen, target, enabled, expires_at, traffic_limit_bytes, created_at, updated_at)
            values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&tunnel.id)
        .bind(&tunnel.client_id)
        .bind(tunnel_mode_to_str(&tunnel.mode))
        .bind(&tunnel.listen)
        .bind(&tunnel.target)
        .bind(bool_to_i64(tunnel.enabled))
        .bind(None::<i64>)
        .bind(None::<i64>)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn insert_config_proxy(
        &self,
        kind: &str,
        proxy: &ProxyListenConfig,
    ) -> anyhow::Result<()> {
        let now = now_secs();
        sqlx::query(
            r#"
            insert or ignore into proxy_listeners
                (kind, listen, client_id, enabled, created_at, updated_at)
            values (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(kind)
        .bind(&proxy.listen)
        .bind(&proxy.client_id)
        .bind(bool_to_i64(proxy.enabled))
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn create_client(&self, input: NewClient) -> anyhow::Result<DbClient> {
        let now = now_secs();
        sqlx::query(
            r#"
            insert into clients
                (id, psk, enabled, remark, max_connections, compress, encrypt, created_at, updated_at)
            values (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&input.id)
        .bind(&input.psk)
        .bind(bool_to_i64(input.enabled))
        .bind(&input.remark)
        .bind(input.max_connections.map(i64::from))
        .bind(bool_to_i64(input.compress))
        .bind(bool_to_i64(input.encrypt))
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        self.get_client(&input.id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("client {} was not inserted", input.id))
    }

    pub async fn get_client(&self, id: &str) -> anyhow::Result<Option<DbClient>> {
        let row = sqlx::query(
            "select id, psk, enabled, remark, max_connections, compress, encrypt from clients where id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_client).transpose()
    }

    pub async fn find_enabled_client_by_id(&self, id: &str) -> anyhow::Result<Option<DbClient>> {
        let row = sqlx::query(
            "select id, psk, enabled, remark, max_connections, compress, encrypt from clients where enabled = 1 and id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_client).transpose()
    }

    pub async fn list_clients(&self) -> anyhow::Result<Vec<DbClient>> {
        let rows = sqlx::query(
            "select id, psk, enabled, remark, max_connections, compress, encrypt from clients order by created_at asc, id asc",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_client).collect()
    }

    pub async fn count_clients(&self) -> anyhow::Result<usize> {
        let row = sqlx::query("select count(*) as count from clients")
            .fetch_one(&self.pool)
            .await?;
        let count: i64 = row.try_get("count")?;
        Ok(count.max(0) as usize)
    }

    pub async fn client_reference_count(&self, id: &str) -> anyhow::Result<usize> {
        let mut count = 0usize;
        for query in [
            "select count(*) as count from tunnels where client_id = ?",
            "select count(*) as count from proxy_listeners where client_id = ?",
            "select count(*) as count from proxy_accounts where client_id = ?",
        ] {
            let row = sqlx::query(query).bind(id).fetch_one(&self.pool).await?;
            let value: i64 = row.try_get("count")?;
            count += value.max(0) as usize;
        }
        Ok(count)
    }

    pub async fn delete_client(&self, id: &str) -> anyhow::Result<bool> {
        let result = sqlx::query("delete from clients where id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn list_tunnels(&self) -> anyhow::Result<Vec<DbTunnel>> {
        let rows = sqlx::query(
            "select id, client_id, mode, listen, target, enabled, expires_at, traffic_limit_bytes, rx_bytes, tx_bytes, disabled_reason from tunnels order by created_at asc, id asc",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_tunnel).collect()
    }

    pub async fn create_tunnel(&self, input: NewTunnel) -> anyhow::Result<DbTunnel> {
        let now = now_secs();
        sqlx::query(
            r#"
            insert into tunnels
                (id, client_id, mode, listen, target, enabled, expires_at, traffic_limit_bytes, created_at, updated_at)
            values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&input.id)
        .bind(&input.client_id)
        .bind(tunnel_mode_to_str(&input.mode))
        .bind(&input.listen)
        .bind(&input.target)
        .bind(bool_to_i64(input.enabled))
        .bind(input.expires_at)
        .bind(input.traffic_limit_bytes.map(u64_to_i64))
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        let row = sqlx::query(
            "select id, client_id, mode, listen, target, enabled, expires_at, traffic_limit_bytes, rx_bytes, tx_bytes, disabled_reason from tunnels where id = ?",
        )
        .bind(&input.id)
        .fetch_one(&self.pool)
        .await?;
        row_to_tunnel(row)
    }

    pub async fn delete_tunnel(&self, id: &str) -> anyhow::Result<bool> {
        let result = sqlx::query("delete from tunnels where id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn disable_tunnel(&self, id: &str, reason: &str) -> anyhow::Result<()> {
        sqlx::query(
            "update tunnels set enabled = 0, disabled_reason = ?, updated_at = ? where id = ?",
        )
        .bind(reason)
        .bind(now_secs())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn count_enabled_tunnels(&self) -> anyhow::Result<usize> {
        let row = sqlx::query("select count(*) as count from tunnels where enabled = 1")
            .fetch_one(&self.pool)
            .await?;
        let count: i64 = row.try_get("count")?;
        Ok(count.max(0) as usize)
    }

    pub async fn get_proxy(&self, kind: &str) -> anyhow::Result<Option<ProxyListenConfig>> {
        let row =
            sqlx::query("select listen, client_id, enabled from proxy_listeners where kind = ?")
                .bind(kind)
                .fetch_optional(&self.pool)
                .await?;
        row.map(row_to_proxy).transpose()
    }

    pub async fn create_proxy_account(
        &self,
        input: NewProxyAccount,
    ) -> anyhow::Result<DbProxyAccount> {
        validate_proxy_kind(&input.kind)?;
        let now = now_secs();
        sqlx::query(
            r#"
            insert into proxy_accounts
                (id, kind, client_id, username, password, enabled, remark, expires_at, traffic_limit_bytes, created_at, updated_at)
            values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&input.id)
        .bind(&input.kind)
        .bind(&input.client_id)
        .bind(&input.username)
        .bind(&input.password)
        .bind(bool_to_i64(input.enabled))
        .bind(&input.remark)
        .bind(input.expires_at)
        .bind(input.traffic_limit_bytes.map(u64_to_i64))
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        self.get_proxy_account(&input.id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("proxy account {} was not inserted", input.id))
    }

    pub async fn get_proxy_account(&self, id: &str) -> anyhow::Result<Option<DbProxyAccount>> {
        let row = sqlx::query(
            r#"
            select id, kind, client_id, username, password, enabled, remark, expires_at, traffic_limit_bytes, rx_bytes, tx_bytes, disabled_reason
            from proxy_accounts
            where id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_proxy_account).transpose()
    }

    pub async fn delete_proxy_account(&self, id: &str) -> anyhow::Result<bool> {
        let result = sqlx::query("delete from proxy_accounts where id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn disable_proxy_account(&self, id: &str, reason: &str) -> anyhow::Result<()> {
        sqlx::query("update proxy_accounts set enabled = 0, disabled_reason = ?, updated_at = ? where id = ?")
            .bind(reason)
            .bind(now_secs())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_proxy_accounts(
        &self,
        kind: Option<&str>,
    ) -> anyhow::Result<Vec<DbProxyAccount>> {
        let rows = if let Some(kind) = kind {
            validate_proxy_kind(kind)?;
            sqlx::query(
                r#"
                select id, kind, client_id, username, password, enabled, remark, expires_at, traffic_limit_bytes, rx_bytes, tx_bytes, disabled_reason
                from proxy_accounts
                where kind = ?
                order by created_at asc, id asc
                "#,
            )
            .bind(kind)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                r#"
                select id, kind, client_id, username, password, enabled, remark, expires_at, traffic_limit_bytes, rx_bytes, tx_bytes, disabled_reason
                from proxy_accounts
                order by kind asc, created_at asc, id asc
                "#,
            )
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter().map(row_to_proxy_account).collect()
    }

    pub async fn has_enabled_proxy_accounts(&self, kind: &str) -> anyhow::Result<bool> {
        validate_proxy_kind(kind)?;
        let row = sqlx::query(
            "select count(*) as count from proxy_accounts where kind = ? and enabled = 1",
        )
        .bind(kind)
        .fetch_one(&self.pool)
        .await?;
        let count: i64 = row.try_get("count")?;
        Ok(count > 0)
    }

    pub async fn find_enabled_proxy_account(
        &self,
        kind: &str,
        username: &str,
        password: &str,
    ) -> anyhow::Result<Option<DbProxyAccount>> {
        validate_proxy_kind(kind)?;
        let row = sqlx::query(
            r#"
            select id, kind, client_id, username, password, enabled, remark, expires_at, traffic_limit_bytes, rx_bytes, tx_bytes, disabled_reason
            from proxy_accounts
            where kind = ? and username = ? and password = ? and enabled = 1
            "#,
        )
        .bind(kind)
        .bind(username)
        .bind(password)
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_proxy_account).transpose()
    }

    pub async fn record_agent_connected(
        &self,
        client_id: &str,
        role: &str,
        remote_addr: &str,
    ) -> anyhow::Result<String> {
        let session_id = Uuid::new_v4().to_string();
        let now = now_secs();
        sqlx::query(
            "insert into agent_sessions (id, client_id, role, remote_addr, connected_at) values (?, ?, ?, ?, ?)",
        )
        .bind(&session_id)
        .bind(client_id)
        .bind(role)
        .bind(remote_addr)
        .bind(now)
        .execute(&self.pool)
        .await?;

        let column = if role == "data" {
            "data_connected_at"
        } else {
            "control_connected_at"
        };
        let sql = format!(
            r#"
            insert into client_online (client_id, online, {column}, last_seen)
            values (?, 1, ?, ?)
            on conflict(client_id) do update set
                online = 1,
                {column} = excluded.{column},
                last_seen = excluded.last_seen
            "#
        );
        sqlx::query(&sql)
            .bind(client_id)
            .bind(now)
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(session_id)
    }

    pub async fn record_agent_disconnected(
        &self,
        session_id: &str,
        client_id: &str,
    ) -> anyhow::Result<()> {
        let now = now_secs();
        sqlx::query("update agent_sessions set disconnected_at = ? where id = ?")
            .bind(now)
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        sqlx::query(
            r#"
            insert into client_online (client_id, online, last_seen)
            values (?, 0, ?)
            on conflict(client_id) do update set online = 0, last_seen = excluded.last_seen
            "#,
        )
        .bind(client_id)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn record_stream_open(
        &self,
        client_id: &str,
        tunnel_id: &str,
        protocol: &TargetProtocol,
        target: &str,
        remote_addr: &str,
    ) -> anyhow::Result<String> {
        let stream_id = Uuid::new_v4().to_string();
        sqlx::query(
            r#"
            insert into stream_sessions
                (id, client_id, tunnel_id, protocol, target, remote_addr, opened_at)
            values (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&stream_id)
        .bind(client_id)
        .bind(tunnel_id)
        .bind(target_protocol_to_str(protocol))
        .bind(target)
        .bind(remote_addr)
        .bind(now_secs())
        .execute(&self.pool)
        .await?;
        Ok(stream_id)
    }

    pub async fn add_traffic(
        &self,
        client_id: &str,
        tunnel_id: &str,
        rx_bytes: u64,
        tx_bytes: u64,
    ) -> anyhow::Result<()> {
        for (scope, key) in [
            ("global", "all"),
            ("client", client_id),
            ("tunnel", tunnel_id),
        ] {
            self.add_traffic_counter(scope, key, rx_bytes, tx_bytes)
                .await?;
        }
        self.add_resource_traffic(tunnel_id, rx_bytes, tx_bytes)
            .await?;
        Ok(())
    }

    async fn add_resource_traffic(
        &self,
        route_id: &str,
        rx_bytes: u64,
        tx_bytes: u64,
    ) -> anyhow::Result<()> {
        let now = now_secs();
        if let Some(account_id) = proxy_account_id_from_route(route_id) {
            sqlx::query(
                r#"
                update proxy_accounts
                set rx_bytes = rx_bytes + ?, tx_bytes = tx_bytes + ?, updated_at = ?
                where id = ?
                "#,
            )
            .bind(u64_to_i64(rx_bytes))
            .bind(u64_to_i64(tx_bytes))
            .bind(now)
            .bind(account_id)
            .execute(&self.pool)
            .await?;
        } else {
            sqlx::query(
                r#"
                update tunnels
                set rx_bytes = rx_bytes + ?, tx_bytes = tx_bytes + ?, updated_at = ?
                where id = ?
                "#,
            )
            .bind(u64_to_i64(rx_bytes))
            .bind(u64_to_i64(tx_bytes))
            .bind(now)
            .bind(route_id)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    async fn add_traffic_counter(
        &self,
        scope: &str,
        key: &str,
        rx_bytes: u64,
        tx_bytes: u64,
    ) -> anyhow::Result<()> {
        let now = now_secs();
        sqlx::query(
            r#"
            insert into traffic_counters (scope, key, rx_bytes, tx_bytes, updated_at)
            values (?, ?, ?, ?, ?)
            on conflict(scope, key) do update set
                rx_bytes = rx_bytes + excluded.rx_bytes,
                tx_bytes = tx_bytes + excluded.tx_bytes,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(scope)
        .bind(key)
        .bind(u64_to_i64(rx_bytes))
        .bind(u64_to_i64(tx_bytes))
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_traffic_counter(
        &self,
        scope: &str,
        key: &str,
    ) -> anyhow::Result<TrafficCounter> {
        let row = sqlx::query(
            "select rx_bytes, tx_bytes from traffic_counters where scope = ? and key = ?",
        )
        .bind(scope)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(TrafficCounter::default());
        };

        let rx_bytes: i64 = row.try_get("rx_bytes")?;
        let tx_bytes: i64 = row.try_get("tx_bytes")?;
        Ok(TrafficCounter {
            rx_bytes: i64_to_u64(rx_bytes),
            tx_bytes: i64_to_u64(tx_bytes),
        })
    }

    pub async fn capture_usage_snapshot(&self) -> anyhow::Result<usize> {
        let now = now_secs();
        let rows = sqlx::query("select scope, key, rx_bytes, tx_bytes from traffic_counters")
            .fetch_all(&self.pool)
            .await?;
        let count = rows.len();
        for row in rows {
            let id = Uuid::new_v4().to_string();
            let scope: String = row.try_get("scope")?;
            let key: String = row.try_get("key")?;
            let rx_bytes: i64 = row.try_get("rx_bytes")?;
            let tx_bytes: i64 = row.try_get("tx_bytes")?;
            sqlx::query(
                r#"
                insert into usage_snapshots
                    (id, scope, key, rx_bytes, tx_bytes, captured_at)
                values (?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(id)
            .bind(scope)
            .bind(key)
            .bind(rx_bytes)
            .bind(tx_bytes)
            .bind(now)
            .execute(&self.pool)
            .await?;
        }
        Ok(count)
    }
}

fn row_to_client(row: sqlx::sqlite::SqliteRow) -> anyhow::Result<DbClient> {
    let max_connections: Option<i64> = row.try_get("max_connections")?;
    Ok(DbClient {
        id: row.try_get("id")?,
        psk: row.try_get("psk")?,
        enabled: i64_to_bool(row.try_get("enabled")?),
        remark: row.try_get("remark")?,
        max_connections: max_connections.map(|value| value.max(0) as u32),
        compress: i64_to_bool(row.try_get("compress")?),
        encrypt: i64_to_bool(row.try_get("encrypt")?),
    })
}

fn row_to_tunnel(row: sqlx::sqlite::SqliteRow) -> anyhow::Result<DbTunnel> {
    let mode: String = row.try_get("mode")?;
    let traffic_limit_bytes: Option<i64> = row.try_get("traffic_limit_bytes")?;
    let rx_bytes: i64 = row.try_get("rx_bytes")?;
    let tx_bytes: i64 = row.try_get("tx_bytes")?;
    Ok(DbTunnel {
        id: row.try_get("id")?,
        client_id: row.try_get("client_id")?,
        mode: str_to_tunnel_mode(&mode)?,
        listen: row.try_get("listen")?,
        target: row.try_get("target")?,
        enabled: i64_to_bool(row.try_get("enabled")?),
        expires_at: row.try_get("expires_at")?,
        traffic_limit_bytes: traffic_limit_bytes.map(i64_to_u64),
        rx_bytes: i64_to_u64(rx_bytes),
        tx_bytes: i64_to_u64(tx_bytes),
        disabled_reason: row.try_get("disabled_reason")?,
    })
}

fn row_to_proxy(row: sqlx::sqlite::SqliteRow) -> anyhow::Result<ProxyListenConfig> {
    Ok(ProxyListenConfig {
        listen: row.try_get("listen")?,
        client_id: row.try_get("client_id")?,
        enabled: i64_to_bool(row.try_get("enabled")?),
    })
}

fn row_to_proxy_account(row: sqlx::sqlite::SqliteRow) -> anyhow::Result<DbProxyAccount> {
    let traffic_limit_bytes: Option<i64> = row.try_get("traffic_limit_bytes")?;
    let rx_bytes: i64 = row.try_get("rx_bytes")?;
    let tx_bytes: i64 = row.try_get("tx_bytes")?;
    Ok(DbProxyAccount {
        id: row.try_get("id")?,
        kind: row.try_get("kind")?,
        client_id: row.try_get("client_id")?,
        username: row.try_get("username")?,
        password: row.try_get("password")?,
        enabled: i64_to_bool(row.try_get("enabled")?),
        remark: row.try_get("remark")?,
        expires_at: row.try_get("expires_at")?,
        traffic_limit_bytes: traffic_limit_bytes.map(i64_to_u64),
        rx_bytes: i64_to_u64(rx_bytes),
        tx_bytes: i64_to_u64(tx_bytes),
        disabled_reason: row.try_get("disabled_reason")?,
    })
}

fn proxy_account_id_from_route(route_id: &str) -> Option<&str> {
    route_id
        .strip_prefix("http-proxy:")
        .or_else(|| route_id.strip_prefix("socks5:"))
        .or_else(|| route_id.strip_prefix("socks5-udp:"))
}

#[cfg(unix)]
async fn secure_database_dir(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).await?;
    Ok(())
}

#[cfg(not(unix))]
async fn secure_database_dir(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(unix)]
async fn secure_database_files(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    for path in [
        path.to_path_buf(),
        sqlite_sidecar_path(path, "-wal"),
        sqlite_sidecar_path(path, "-shm"),
    ] {
        if tokio::fs::try_exists(&path).await? {
            tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).await?;
        }
    }

    Ok(())
}

#[cfg(not(unix))]
async fn secure_database_files(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn sqlite_sidecar_path(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut sidecar = path.as_os_str().to_os_string();
    sidecar.push(suffix);
    std::path::PathBuf::from(sidecar)
}

fn validate_proxy_kind(kind: &str) -> anyhow::Result<()> {
    match kind {
        "http" | "socks5" => Ok(()),
        value => anyhow::bail!("invalid proxy account kind {value}"),
    }
}

fn tunnel_mode_to_str(mode: &TunnelMode) -> &'static str {
    match mode {
        TunnelMode::Tcp => "tcp",
        TunnelMode::Udp => "udp",
    }
}

fn target_protocol_to_str(protocol: &TargetProtocol) -> &'static str {
    match protocol {
        TargetProtocol::Tcp => "tcp",
        TargetProtocol::Udp => "udp",
    }
}

fn str_to_tunnel_mode(mode: &str) -> anyhow::Result<TunnelMode> {
    match mode {
        "tcp" => Ok(TunnelMode::Tcp),
        "udp" => Ok(TunnelMode::Udp),
        value => anyhow::bail!("invalid tunnel mode {value}"),
    }
}

fn bool_to_i64(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

fn i64_to_bool(value: i64) -> bool {
    value != 0
}

fn u64_to_i64(value: u64) -> i64 {
    value.min(i64::MAX as u64) as i64
}

fn i64_to_u64(value: i64) -> u64 {
    value.max(0) as u64
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}
