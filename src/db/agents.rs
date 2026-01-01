//! Agent CRUD operations
//!
//! Agents are the unified model for humans, models, MCP clients, and bots.

use super::{new_id, now_ms, Database};
use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Agent kind discriminator
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Human,
    Model,
    McpClient,
    Bot,
}

impl AgentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentKind::Human => "human",
            AgentKind::Model => "model",
            AgentKind::McpClient => "mcp_client",
            AgentKind::Bot => "bot",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "human" => Some(AgentKind::Human),
            "model" => Some(AgentKind::Model),
            "mcp_client" => Some(AgentKind::McpClient),
            "bot" => Some(AgentKind::Bot),
            _ => None,
        }
    }
}

/// Backend kind for model agents
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    Ollama,
    OpenAI,
    Anthropic,
    LlamaCpp,
    Gemini,
}

impl BackendKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            BackendKind::Ollama => "ollama",
            BackendKind::OpenAI => "openai",
            BackendKind::Anthropic => "anthropic",
            BackendKind::LlamaCpp => "llamacpp",
            BackendKind::Gemini => "gemini",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ollama" => Some(BackendKind::Ollama),
            "openai" => Some(BackendKind::OpenAI),
            "anthropic" => Some(BackendKind::Anthropic),
            "llamacpp" => Some(BackendKind::LlamaCpp),
            "gemini" => Some(BackendKind::Gemini),
            _ => None,
        }
    }
}

/// An agent in the system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub kind: AgentKind,
    pub capabilities: Vec<String>,
    pub created_at: i64,

    // Lua code storage
    pub hud_script: Option<String>,
    pub wrap_script: Option<String>,
    pub context_format: String,

    // Model backend (None for humans/bots)
    pub backend_kind: Option<BackendKind>,
    pub backend_model_id: Option<String>,
    pub backend_endpoint: Option<String>,
    pub backend_config: Option<String>,
    pub system_prompt: Option<String>,
}

impl Agent {
    /// Create a new agent with defaults
    pub fn new(name: impl Into<String>, kind: AgentKind) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            display_name: None,
            kind,
            capabilities: vec![],
            created_at: now_ms(),
            hud_script: None,
            wrap_script: None,
            context_format: "markdown".to_string(),
            backend_kind: None,
            backend_model_id: None,
            backend_endpoint: None,
            backend_config: None,
            system_prompt: None,
        }
    }

    /// Check if agent has a specific capability
    pub fn has_capability(&self, cap: &str) -> bool {
        self.capabilities.iter().any(|c| c == cap)
    }
}

/// Session kind discriminator
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Ssh,
    Mcp,
    Api,
    Internal,
}

impl SessionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionKind::Ssh => "ssh",
            SessionKind::Mcp => "mcp",
            SessionKind::Api => "api",
            SessionKind::Internal => "internal",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ssh" => Some(SessionKind::Ssh),
            "mcp" => Some(SessionKind::Mcp),
            "api" => Some(SessionKind::Api),
            "internal" => Some(SessionKind::Internal),
            _ => None,
        }
    }
}

/// An active or historical session for an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSession {
    pub id: String,
    pub agent_id: String,
    pub kind: SessionKind,
    pub connected_at: i64,
    pub disconnected_at: Option<i64>,
    pub metadata: Option<String>,
}

impl AgentSession {
    pub fn new(agent_id: impl Into<String>, kind: SessionKind) -> Self {
        Self {
            id: new_id(),
            agent_id: agent_id.into(),
            kind,
            connected_at: now_ms(),
            disconnected_at: None,
            metadata: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.disconnected_at.is_none()
    }
}

/// Auth kind discriminator
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    Pubkey,
    ApiKey,
    McpToken,
    Local,
}

impl AuthKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthKind::Pubkey => "pubkey",
            AuthKind::ApiKey => "api_key",
            AuthKind::McpToken => "mcp_token",
            AuthKind::Local => "local",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pubkey" => Some(AuthKind::Pubkey),
            "api_key" => Some(AuthKind::ApiKey),
            "mcp_token" => Some(AuthKind::McpToken),
            "local" => Some(AuthKind::Local),
            _ => None,
        }
    }
}

/// Authentication credential for an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentAuth {
    pub agent_id: String,
    pub kind: AuthKind,
    pub auth_data: String,
    pub created_at: i64,
}

impl AgentAuth {
    pub fn new(agent_id: impl Into<String>, kind: AuthKind, auth_data: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            kind,
            auth_data: auth_data.into(),
            created_at: now_ms(),
        }
    }
}

// Database operations
impl Database {
    // --- Agent CRUD ---

    /// Insert a new agent
    pub fn insert_agent(&self, agent: &Agent) -> Result<()> {
        let conn = self.conn()?;
        let caps_json = serde_json::to_string(&agent.capabilities)?;
        let backend_kind = agent.backend_kind.as_ref().map(|k| k.as_str());

        conn.execute(
            r#"
            INSERT INTO agents (
                id, name, display_name, agent_kind, capabilities, created_at,
                hud_script, wrap_script, context_format,
                backend_kind, backend_model_id, backend_endpoint, backend_config, system_prompt
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            "#,
            params![
                agent.id,
                agent.name,
                agent.display_name,
                agent.kind.as_str(),
                caps_json,
                agent.created_at,
                agent.hud_script,
                agent.wrap_script,
                agent.context_format,
                backend_kind,
                agent.backend_model_id,
                agent.backend_endpoint,
                agent.backend_config,
                agent.system_prompt,
            ],
        )
        .context("failed to insert agent")?;
        Ok(())
    }

    /// Get agent by ID
    pub fn get_agent(&self, id: &str) -> Result<Option<Agent>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, name, display_name, agent_kind, capabilities, created_at,
                   hud_script, wrap_script, context_format,
                   backend_kind, backend_model_id, backend_endpoint, backend_config, system_prompt
            FROM agents WHERE id = ?1
            "#,
            )
            .context("failed to prepare agent query")?;

        let agent = stmt
            .query_row(params![id], Self::agent_from_row)
            .optional()
            .context("failed to query agent")?;

        Ok(agent)
    }

    /// Get agent by name
    pub fn get_agent_by_name(&self, name: &str) -> Result<Option<Agent>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, name, display_name, agent_kind, capabilities, created_at,
                   hud_script, wrap_script, context_format,
                   backend_kind, backend_model_id, backend_endpoint, backend_config, system_prompt
            FROM agents WHERE name = ?1
            "#,
            )
            .context("failed to prepare agent query")?;

        let agent = stmt
            .query_row(params![name], Self::agent_from_row)
            .optional()
            .context("failed to query agent by name")?;

        Ok(agent)
    }

    /// List all agents, optionally filtered by kind
    pub fn list_agents(&self, kind: Option<AgentKind>) -> Result<Vec<Agent>> {
        let conn = self.conn()?;
        let sql = match kind {
            Some(_) => {
                r#"
                SELECT id, name, display_name, agent_kind, capabilities, created_at,
                       hud_script, wrap_script, context_format,
                       backend_kind, backend_model_id, backend_endpoint, backend_config, system_prompt
                FROM agents WHERE agent_kind = ?1 ORDER BY name
            "#
            }
            None => {
                r#"
                SELECT id, name, display_name, agent_kind, capabilities, created_at,
                       hud_script, wrap_script, context_format,
                       backend_kind, backend_model_id, backend_endpoint, backend_config, system_prompt
                FROM agents ORDER BY name
            "#
            }
        };

        let mut stmt = conn
            .prepare(sql)
            .context("failed to prepare agents query")?;
        let rows = match kind {
            Some(k) => stmt.query(params![k.as_str()])?,
            None => stmt.query([])?,
        };

        let agents = rows
            .mapped(Self::agent_from_row)
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list agents")?;

        Ok(agents)
    }

    /// Update an agent
    pub fn update_agent(&self, agent: &Agent) -> Result<()> {
        let conn = self.conn()?;
        let caps_json = serde_json::to_string(&agent.capabilities)?;
        let backend_kind = agent.backend_kind.as_ref().map(|k| k.as_str());

        conn.execute(
            r#"
            UPDATE agents SET
                name = ?2, display_name = ?3, agent_kind = ?4, capabilities = ?5,
                hud_script = ?6, wrap_script = ?7, context_format = ?8,
                backend_kind = ?9, backend_model_id = ?10, backend_endpoint = ?11,
                backend_config = ?12, system_prompt = ?13
            WHERE id = ?1
            "#,
            params![
                agent.id,
                agent.name,
                agent.display_name,
                agent.kind.as_str(),
                caps_json,
                agent.hud_script,
                agent.wrap_script,
                agent.context_format,
                backend_kind,
                agent.backend_model_id,
                agent.backend_endpoint,
                agent.backend_config,
                agent.system_prompt,
            ],
        )
        .context("failed to update agent")?;
        Ok(())
    }

    /// Delete an agent
    pub fn delete_agent(&self, id: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM agents WHERE id = ?1", params![id])
            .context("failed to delete agent")?;
        Ok(())
    }

    fn agent_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Agent> {
        let caps_json: String = row.get(4)?;
        let capabilities: Vec<String> = serde_json::from_str(&caps_json).unwrap_or_default();
        let kind_str: String = row.get(3)?;
        let backend_kind_str: Option<String> = row.get(9)?;

        Ok(Agent {
            id: row.get(0)?,
            name: row.get(1)?,
            display_name: row.get(2)?,
            kind: AgentKind::parse(&kind_str).unwrap_or(AgentKind::Human),
            capabilities,
            created_at: row.get(5)?,
            hud_script: row.get(6)?,
            wrap_script: row.get(7)?,
            context_format: row.get(8)?,
            backend_kind: backend_kind_str.and_then(|s| BackendKind::parse(&s)),
            backend_model_id: row.get(10)?,
            backend_endpoint: row.get(11)?,
            backend_config: row.get(12)?,
            system_prompt: row.get(13)?,
        })
    }

    // --- Session CRUD ---

    /// Insert a new session
    pub fn insert_session(&self, session: &AgentSession) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"
            INSERT INTO agent_sessions (id, agent_id, session_kind, connected_at, disconnected_at, metadata)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![
                session.id,
                session.agent_id,
                session.kind.as_str(),
                session.connected_at,
                session.disconnected_at,
                session.metadata,
            ],
        )
        .context("failed to insert session")?;
        Ok(())
    }

    /// Get session by ID
    pub fn get_session(&self, id: &str) -> Result<Option<AgentSession>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, agent_id, session_kind, connected_at, disconnected_at, metadata
            FROM agent_sessions WHERE id = ?1
            "#,
            )
            .context("failed to prepare session query")?;

        let session = stmt
            .query_row(params![id], Self::session_from_row)
            .optional()
            .context("failed to query session")?;

        Ok(session)
    }

    /// List active sessions for an agent
    pub fn list_active_sessions(&self, agent_id: &str) -> Result<Vec<AgentSession>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, agent_id, session_kind, connected_at, disconnected_at, metadata
            FROM agent_sessions
            WHERE agent_id = ?1 AND disconnected_at IS NULL
            ORDER BY connected_at DESC
            "#,
            )
            .context("failed to prepare sessions query")?;

        let sessions = stmt
            .query(params![agent_id])?
            .mapped(Self::session_from_row)
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list sessions")?;

        Ok(sessions)
    }

    /// Mark a session as disconnected
    pub fn disconnect_session(&self, id: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE agent_sessions SET disconnected_at = ?2 WHERE id = ?1",
            params![id, now_ms()],
        )
        .context("failed to disconnect session")?;
        Ok(())
    }

    fn session_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentSession> {
        let kind_str: String = row.get(2)?;
        Ok(AgentSession {
            id: row.get(0)?,
            agent_id: row.get(1)?,
            kind: SessionKind::parse(&kind_str).unwrap_or(SessionKind::Internal),
            connected_at: row.get(3)?,
            disconnected_at: row.get(4)?,
            metadata: row.get(5)?,
        })
    }

    // --- Auth CRUD ---

    /// Insert or replace auth credential
    pub fn upsert_auth(&self, auth: &AgentAuth) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"
            INSERT OR REPLACE INTO agent_auth (agent_id, auth_kind, auth_data, created_at)
            VALUES (?1, ?2, ?3, ?4)
            "#,
            params![
                auth.agent_id,
                auth.kind.as_str(),
                auth.auth_data,
                auth.created_at,
            ],
        )
        .context("failed to upsert auth")?;
        Ok(())
    }

    /// Get auth by agent and kind
    pub fn get_auth(&self, agent_id: &str, kind: AuthKind) -> Result<Option<AgentAuth>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT agent_id, auth_kind, auth_data, created_at
            FROM agent_auth WHERE agent_id = ?1 AND auth_kind = ?2
            "#,
            )
            .context("failed to prepare auth query")?;

        let auth = stmt
            .query_row(params![agent_id, kind.as_str()], |row| {
                let kind_str: String = row.get(1)?;
                Ok(AgentAuth {
                    agent_id: row.get(0)?,
                    kind: AuthKind::parse(&kind_str).unwrap_or(AuthKind::Local),
                    auth_data: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })
            .optional()
            .context("failed to query auth")?;

        Ok(auth)
    }

    /// Find agent by auth data (e.g., pubkey fingerprint)
    pub fn find_agent_by_auth(&self, kind: AuthKind, auth_data: &str) -> Result<Option<Agent>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT a.id, a.name, a.display_name, a.agent_kind, a.capabilities, a.created_at,
                   a.hud_script, a.wrap_script, a.context_format,
                   a.backend_kind, a.backend_model_id, a.backend_endpoint, a.backend_config, a.system_prompt
            FROM agents a
            JOIN agent_auth aa ON a.id = aa.agent_id
            WHERE aa.auth_kind = ?1 AND aa.auth_data = ?2
            "#,
            )
            .context("failed to prepare agent by auth query")?;

        let agent = stmt
            .query_row(params![kind.as_str(), auth_data], |row| {
                Self::agent_from_row(row)
            })
            .optional()
            .context("failed to query agent by auth")?;

        Ok(agent)
    }

    /// Delete auth credential
    pub fn delete_auth(&self, agent_id: &str, kind: AuthKind) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "DELETE FROM agent_auth WHERE agent_id = ?1 AND auth_kind = ?2",
            params![agent_id, kind.as_str()],
        )
        .context("failed to delete auth")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_crud() -> Result<()> {
        let db = Database::in_memory()?;

        let mut agent = Agent::new("alice", AgentKind::Human);
        agent.display_name = Some("Alice".to_string());
        agent.capabilities = vec!["chat".to_string(), "navigation".to_string()];

        db.insert_agent(&agent)?;

        let fetched = db.get_agent(&agent.id)?.expect("agent should exist");
        assert_eq!(fetched.name, "alice");
        assert_eq!(fetched.display_name, Some("Alice".to_string()));
        assert_eq!(fetched.kind, AgentKind::Human);
        assert!(fetched.has_capability("chat"));
        assert!(!fetched.has_capability("admin"));

        let by_name = db.get_agent_by_name("alice")?.expect("should find by name");
        assert_eq!(by_name.id, agent.id);

        let all = db.list_agents(None)?;
        assert_eq!(all.len(), 1);

        let humans = db.list_agents(Some(AgentKind::Human))?;
        assert_eq!(humans.len(), 1);

        let models = db.list_agents(Some(AgentKind::Model))?;
        assert_eq!(models.len(), 0);

        db.delete_agent(&agent.id)?;
        assert!(db.get_agent(&agent.id)?.is_none());

        Ok(())
    }

    #[test]
    fn test_session_lifecycle() -> Result<()> {
        let db = Database::in_memory()?;

        let agent = Agent::new("bob", AgentKind::Human);
        db.insert_agent(&agent)?;

        let session = AgentSession::new(&agent.id, SessionKind::Ssh);
        db.insert_session(&session)?;

        let active = db.list_active_sessions(&agent.id)?;
        assert_eq!(active.len(), 1);
        assert!(active[0].is_active());

        db.disconnect_session(&session.id)?;

        let active = db.list_active_sessions(&agent.id)?;
        assert_eq!(active.len(), 0);

        let fetched = db.get_session(&session.id)?.expect("session should exist");
        assert!(!fetched.is_active());

        Ok(())
    }

    #[test]
    fn test_auth_lookup() -> Result<()> {
        let db = Database::in_memory()?;

        let agent = Agent::new("carol", AgentKind::Human);
        db.insert_agent(&agent)?;

        let auth = AgentAuth::new(&agent.id, AuthKind::Pubkey, "SHA256:abc123");
        db.upsert_auth(&auth)?;

        let found = db
            .find_agent_by_auth(AuthKind::Pubkey, "SHA256:abc123")?
            .expect("should find by auth");
        assert_eq!(found.name, "carol");

        let not_found = db.find_agent_by_auth(AuthKind::Pubkey, "SHA256:xyz789")?;
        assert!(not_found.is_none());

        Ok(())
    }
}
