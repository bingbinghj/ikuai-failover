use async_trait::async_trait;
use serde_json::Value;

use crate::{config::IkuaiConfig, policy::Resource, web::WebClient};

#[async_trait]
pub trait RouterApi: Send + Sync {
    async fn get(&self, path: &str) -> Result<Value, String>;
    async fn put(&self, resource: Resource, body: &Value) -> Result<(), String>;
    fn connection_configured(&self) -> bool {
        true
    }
    fn ipv6_supported(&self) -> bool {
        false
    }
}

pub struct Client {
    web: WebClient,
}

pub struct DisabledClient;

#[async_trait]
impl RouterApi for DisabledClient {
    fn connection_configured(&self) -> bool {
        false
    }
    async fn get(&self, _: &str) -> Result<Value, String> {
        Err("WAN management is disabled".into())
    }
    async fn put(&self, _: Resource, _: &Value) -> Result<(), String> {
        Err("WAN management is disabled".into())
    }
}

impl Client {
    pub fn with_web_credentials(
        config: &IkuaiConfig,
        username: String,
        password: String,
    ) -> Result<Self, String> {
        Ok(Self {
            web: WebClient::with_credentials(config, username, password)?,
        })
    }

    pub fn new(config: &IkuaiConfig) -> Result<Self, String> {
        Ok(Self {
            web: WebClient::new(config)?,
        })
    }
}

#[async_trait]
impl RouterApi for Client {
    async fn get(&self, path: &str) -> Result<Value, String> {
        self.web.get(path).await
    }

    async fn put(&self, resource: Resource, body: &Value) -> Result<(), String> {
        self.web.put(resource, body).await
    }

    fn ipv6_supported(&self) -> bool {
        true
    }
}
