pub mod image_registry;
pub mod labels;
pub mod resource_limits;
pub mod topology_spread;

use json_patch::PatchOperation;
use kube::core::admission::AdmissionRequest;
use kube::core::DynamicObject;
use serde_json::Value;

pub struct PolicyOutput {
    pub violations: Vec<String>,
    pub patches: Vec<PatchOperation>,
}

impl PolicyOutput {
    pub fn allowed() -> Self {
        Self {
            violations: Vec::new(),
            patches: Vec::new(),
        }
    }
}

pub fn get_pod_spec<'a>(data: &'a Value, kind: &str) -> Option<&'a Value> {
    match kind {
        "Pod" => data.get("spec"),
        "Deployment" | "ReplicaSet" | "StatefulSet" | "DaemonSet" | "Job" => {
            data.get("spec")?.get("template")?.get("spec")
        }
        "CronJob" => data
            .get("spec")?
            .get("jobTemplate")?
            .get("spec")?
            .get("template")?
            .get("spec"),
        _ => None,
    }
}

pub fn spec_prefix(kind: &str) -> &str {
    match kind {
        "Pod" => "spec",
        "CronJob" => "spec/jobTemplate/spec/template/spec",
        _ => "spec/template/spec",
    }
}

pub fn get_containers(pod_spec: &Value) -> Vec<(usize, &Value)> {
    pod_spec
        .get("containers")
        .and_then(|c| c.as_array())
        .map(|arr| arr.iter().enumerate().collect())
        .unwrap_or_default()
}

pub fn container_name(container: &Value) -> &str {
    container
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("<unnamed>")
}

pub fn resource_name<'a>(request: &'a AdmissionRequest<DynamicObject>, object: &'a DynamicObject) -> &'a str {
    if request.name.is_empty() {
        object
            .metadata
            .generate_name
            .as_deref()
            .unwrap_or("<unknown>")
    } else {
        &request.name
    }
}
