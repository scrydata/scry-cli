//! Shadow reference resolution.

use crate::client::ApiClient;
use crate::error::CliError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Response from shadow resolution endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveResponse {
    pub shadow_id: Uuid,
    pub source_name: String,
    pub shadow_name: String,
}

/// Response from shadow detail endpoint (for UUID lookups).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShadowDetailResponse {
    pub id: Uuid,
    pub name: Option<String>,
    pub source_name: Option<String>,
}

/// Parsed shadow reference: either UUID or source/shadow-name
#[derive(Debug, Clone)]
pub enum ShadowRef {
    Id(Uuid),
    Named { source: String, shadow: String },
}

impl ShadowRef {
    /// Parse a shadow reference string.
    /// Accepts: UUID, or "source/shadow-name"
    pub fn parse(s: &str) -> Result<Self, CliError> {
        // Try UUID first
        if let Ok(uuid) = s.parse::<Uuid>() {
            return Ok(ShadowRef::Id(uuid));
        }

        // Try source/shadow format
        if let Some((source, shadow)) = s.split_once('/') {
            if source.is_empty() || shadow.is_empty() {
                return Err(CliError::Other(format!(
                    "Invalid shadow reference '{}': source and shadow name cannot be empty",
                    s
                )));
            }
            return Ok(ShadowRef::Named {
                source: source.to_string(),
                shadow: shadow.to_string(),
            });
        }

        Err(CliError::Other(format!(
            "Invalid shadow reference '{}': expected UUID or 'source/shadow-name'",
            s
        )))
    }

    /// Resolve this reference to a shadow UUID.
    /// If already a UUID, returns it directly.
    /// If named, calls the API to resolve.
    pub async fn resolve(&self, client: &ApiClient) -> Result<Uuid, CliError> {
        match self {
            ShadowRef::Id(uuid) => Ok(*uuid),
            ShadowRef::Named { source, shadow } => {
                let path = format!("/api/v1/shadows/by-name/{}/{}", source, shadow);
                let response: ResolveResponse = client.get(&path).await?;
                Ok(response.shadow_id)
            }
        }
    }

    /// Resolve and return full info (for display purposes).
    #[allow(dead_code)]
    pub async fn resolve_full(&self, client: &ApiClient) -> Result<ResolveResponse, CliError> {
        match self {
            ShadowRef::Id(uuid) => {
                // For UUID, we need to fetch the shadow to get names
                let path = format!("/api/v1/shadows/{}", uuid);
                let shadow: ShadowDetailResponse = client.get(&path).await?;
                Ok(ResolveResponse {
                    shadow_id: *uuid,
                    source_name: shadow.source_name.unwrap_or_default(),
                    shadow_name: shadow.name.unwrap_or_default(),
                })
            }
            ShadowRef::Named { source, shadow } => {
                let path = format!("/api/v1/shadows/by-name/{}/{}", source, shadow);
                client.get(&path).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_uuid() {
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        match ShadowRef::parse(uuid).unwrap() {
            ShadowRef::Id(id) => assert_eq!(id.to_string(), uuid),
            _ => panic!("Expected Id variant"),
        }
    }

    #[test]
    fn test_parse_named() {
        match ShadowRef::parse("prod-db/ci-main").unwrap() {
            ShadowRef::Named { source, shadow } => {
                assert_eq!(source, "prod-db");
                assert_eq!(shadow, "ci-main");
            }
            _ => panic!("Expected Named variant"),
        }
    }

    #[test]
    fn test_parse_invalid_empty_parts() {
        assert!(ShadowRef::parse("/ci-main").is_err());
        assert!(ShadowRef::parse("prod-db/").is_err());
    }

    #[test]
    fn test_parse_invalid_format() {
        assert!(ShadowRef::parse("not-a-uuid-or-path").is_err());
    }
}
