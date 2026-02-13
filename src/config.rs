use std::fmt;

use figment::{Figment, providers::{Env, Format, Yaml}};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyMode {
    Enforce,
    Warn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PolicyName {
    ResourceLimits,
    ImageRegistry,
    Labels,
    TopologySpread,
}

impl PolicyName {
    pub const ALL: [PolicyName; 4] = [
        PolicyName::ResourceLimits,
        PolicyName::ImageRegistry,
        PolicyName::Labels,
        PolicyName::TopologySpread,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            PolicyName::ResourceLimits => "resource_limits",
            PolicyName::ImageRegistry => "image_registry",
            PolicyName::Labels => "labels",
            PolicyName::TopologySpread => "topology_spread",
        }
    }
}

impl fmt::Display for PolicyName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequiredLabel {
    pub key: String,
    pub pattern: Option<String>,
}

fn default_listen_addr() -> String {
    "0.0.0.0:8443".to_string()
}

fn default_tls_cert_path() -> String {
    "/certs/tls.crt".to_string()
}

fn default_tls_key_path() -> String {
    "/certs/tls.key".to_string()
}

fn default_metrics_addr() -> String {
    "0.0.0.0:9090".to_string()
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_cpu_request() -> String {
    "100m".to_string()
}

fn default_cpu_limit() -> String {
    "500m".to_string()
}

fn default_memory_request() -> String {
    "128Mi".to_string()
}

fn default_memory_limit() -> String {
    "512Mi".to_string()
}

fn default_max_skew() -> i32 {
    1
}

fn default_topology_key() -> String {
    "topology.kubernetes.io/zone".to_string()
}

fn default_when_unsatisfiable() -> String {
    "DoNotSchedule".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentinelConfig {
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,
    #[serde(default = "default_tls_cert_path")]
    pub tls_cert_path: String,
    #[serde(default = "default_tls_key_path")]
    pub tls_key_path: String,
    #[serde(default = "default_metrics_addr")]
    pub metrics_addr: String,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    pub policies: PoliciesConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoliciesConfig {
    #[serde(rename = "enforce_resource_limits")]
    pub resource_limits: ResourceLimitsPolicy,
    #[serde(rename = "allowed_registries")]
    pub image_registry: AllowedRegistriesPolicy,
    #[serde(rename = "required_labels")]
    pub labels: RequiredLabelsPolicy,
    pub topology_spread: TopologySpreadPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimitsPolicy {
    pub enabled: bool,
    pub mode: PolicyMode,
    pub max_cpu_millicores: Option<u64>,
    pub max_memory_mb: Option<u64>,
    #[serde(default)]
    pub inject_defaults: bool,
    #[serde(default = "default_cpu_request")]
    pub default_cpu_request: String,
    #[serde(default = "default_cpu_limit")]
    pub default_cpu_limit: String,
    #[serde(default = "default_memory_request")]
    pub default_memory_request: String,
    #[serde(default = "default_memory_limit")]
    pub default_memory_limit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllowedRegistriesPolicy {
    pub enabled: bool,
    pub mode: PolicyMode,
    pub registries: Vec<String>,
    #[serde(default)]
    pub allow_latest_tag: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequiredLabelsPolicy {
    pub enabled: bool,
    pub mode: PolicyMode,
    pub labels: Vec<RequiredLabel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologySpreadPolicy {
    pub enabled: bool,
    pub mode: PolicyMode,
    #[serde(default = "default_max_skew")]
    pub max_skew: i32,
    #[serde(default = "default_topology_key")]
    pub topology_key: String,
    #[serde(default = "default_when_unsatisfiable")]
    pub when_unsatisfiable: String,
    #[serde(default)]
    pub inject_if_missing: bool,
}

impl PoliciesConfig {
    pub fn policy_mode(&self, name: PolicyName) -> &PolicyMode {
        match name {
            PolicyName::ResourceLimits => &self.resource_limits.mode,
            PolicyName::ImageRegistry => &self.image_registry.mode,
            PolicyName::Labels => &self.labels.mode,
            PolicyName::TopologySpread => &self.topology_spread.mode,
        }
    }

    pub fn policy_enabled(&self, name: PolicyName) -> bool {
        match name {
            PolicyName::ResourceLimits => self.resource_limits.enabled,
            PolicyName::ImageRegistry => self.image_registry.enabled,
            PolicyName::Labels => self.labels.enabled,
            PolicyName::TopologySpread => self.topology_spread.enabled,
        }
    }
}

impl SentinelConfig {
    pub fn load(path: &str) -> Result<Self, Box<figment::Error>> {
        Figment::new()
            .merge(Yaml::file(path))
            .merge(Env::prefixed("SENTINEL_").split("__"))
            .extract()
            .map_err(Box::new)
    }
}
