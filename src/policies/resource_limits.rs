use json_patch::jsonptr::PointerBuf;
use json_patch::{AddOperation, PatchOperation};
use kube::core::admission::AdmissionRequest;
use kube::core::DynamicObject;
use serde_json::{json, Value};

use crate::config::ResourceLimitsPolicy;

use super::{container_name, get_containers, get_pod_spec, spec_prefix, PolicyOutput};

pub fn evaluate(
    config: &ResourceLimitsPolicy,
    request: &AdmissionRequest<DynamicObject>,
    mutating: bool,
) -> PolicyOutput {
    let object = match &request.object {
        Some(obj) => obj,
        None => return PolicyOutput::allowed(),
    };

    let kind = &request.kind.kind;
    let pod_spec = match get_pod_spec(&object.data, kind) {
        Some(spec) => spec,
        None => return PolicyOutput::allowed(),
    };

    let containers = get_containers(pod_spec);
    let prefix = spec_prefix(kind);
    let mut violations = Vec::new();
    let mut patches = Vec::new();

    for (i, container) in &containers {
        let name = container_name(container);
        let resources = container.get("resources");

        let has_requests = resources
            .and_then(|r| r.get("requests"))
            .and_then(|r| r.as_object())
            .is_some_and(|m| !m.is_empty());
        let has_limits = resources
            .and_then(|r| r.get("limits"))
            .and_then(|r| r.as_object())
            .is_some_and(|m| !m.is_empty());

        if !has_requests || !has_limits {
            // Skip this violation in mutate path if inject_defaults will fix it
            let will_be_patched = mutating && config.inject_defaults;
            if !will_be_patched {
                let missing = match (has_requests, has_limits) {
                    (false, false) => "requests and limits",
                    (false, true) => "requests",
                    (true, false) => "limits",
                    _ => unreachable!(),
                };
                violations.push(format!(
                    "container '{name}' missing resource {missing}"
                ));
            }
        }

        if let Some(max_cpu) = config.max_cpu_millicores {
            for section in &["requests", "limits"] {
                if let Some(cpu_str) = resources
                    .and_then(|r| r.get(*section))
                    .and_then(|s| s.get("cpu"))
                    .and_then(|v| v.as_str())
                {
                    if let Some(cpu_m) = parse_cpu_millicores(cpu_str) {
                        if cpu_m > max_cpu {
                            violations.push(format!(
                                "container '{name}' {section} cpu '{cpu_str}' ({cpu_m}m) \
                                 exceeds maximum allowed {max_cpu}m"
                            ));
                        }
                    }
                }
            }
        }

        if let Some(max_mem_mb) = config.max_memory_mb {
            let max_mem_bytes = max_mem_mb * 1024 * 1024;
            for section in &["requests", "limits"] {
                if let Some(mem_str) = resources
                    .and_then(|r| r.get(*section))
                    .and_then(|s| s.get("memory"))
                    .and_then(|v| v.as_str())
                {
                    if let Some(mem_bytes) = parse_memory_bytes(mem_str) {
                        if mem_bytes > max_mem_bytes {
                            violations.push(format!(
                                "container '{name}' {section} memory '{mem_str}' \
                                 ({} Mi) exceeds maximum allowed {max_mem_mb} Mi",
                                mem_bytes / (1024 * 1024)
                            ));
                        }
                    }
                }
            }
        }

        if config.inject_defaults {
            generate_resource_patches(config, container, prefix, *i, &mut patches);
        }
    }

    PolicyOutput {
        violations,
        patches,
    }
}

fn generate_resource_patches(
    config: &ResourceLimitsPolicy,
    container: &Value,
    prefix: &str,
    idx: usize,
    patches: &mut Vec<PatchOperation>,
) {
    let resources = container.get("resources");
    let has_resources = resources
        .and_then(|r| r.as_object())
        .is_some_and(|m| !m.is_empty());

    let idx_str = idx.to_string();

    if !has_resources {
        let mut path_parts: Vec<&str> = prefix.split('/').collect();
        path_parts.extend_from_slice(&["containers", &idx_str, "resources"]);
        patches.push(PatchOperation::Add(AddOperation {
            path: PointerBuf::from_tokens(path_parts),
            value: json!({
                "requests": {
                    "cpu": config.default_cpu_request,
                    "memory": config.default_memory_request,
                },
                "limits": {
                    "cpu": config.default_cpu_limit,
                    "memory": config.default_memory_limit,
                }
            }),
        }));
        return;
    }

    let resources = resources.unwrap();

    let has_requests = resources.get("requests").is_some();
    if !has_requests {
        let mut path_parts: Vec<&str> = prefix.split('/').collect();
        path_parts.extend_from_slice(&["containers", &idx_str, "resources", "requests"]);
        patches.push(PatchOperation::Add(AddOperation {
            path: PointerBuf::from_tokens(path_parts),
            value: json!({
                "cpu": config.default_cpu_request,
                "memory": config.default_memory_request,
            }),
        }));
    } else {
        let requests = &resources["requests"];
        if requests.get("cpu").is_none() {
            let mut path_parts: Vec<&str> = prefix.split('/').collect();
            path_parts
                .extend_from_slice(&["containers", &idx_str, "resources", "requests", "cpu"]);
            patches.push(PatchOperation::Add(AddOperation {
                path: PointerBuf::from_tokens(path_parts),
                value: Value::String(config.default_cpu_request.clone()),
            }));
        }
        if requests.get("memory").is_none() {
            let mut path_parts: Vec<&str> = prefix.split('/').collect();
            path_parts.extend_from_slice(&[
                "containers",
                &idx_str,
                "resources",
                "requests",
                "memory",
            ]);
            patches.push(PatchOperation::Add(AddOperation {
                path: PointerBuf::from_tokens(path_parts),
                value: Value::String(config.default_memory_request.clone()),
            }));
        }
    }

    let has_limits = resources.get("limits").is_some();
    if !has_limits {
        let mut path_parts: Vec<&str> = prefix.split('/').collect();
        path_parts.extend_from_slice(&["containers", &idx_str, "resources", "limits"]);
        patches.push(PatchOperation::Add(AddOperation {
            path: PointerBuf::from_tokens(path_parts),
            value: json!({
                "cpu": config.default_cpu_limit,
                "memory": config.default_memory_limit,
            }),
        }));
    } else {
        let limits = &resources["limits"];
        if limits.get("cpu").is_none() {
            let mut path_parts: Vec<&str> = prefix.split('/').collect();
            path_parts
                .extend_from_slice(&["containers", &idx_str, "resources", "limits", "cpu"]);
            patches.push(PatchOperation::Add(AddOperation {
                path: PointerBuf::from_tokens(path_parts),
                value: Value::String(config.default_cpu_limit.clone()),
            }));
        }
        if limits.get("memory").is_none() {
            let mut path_parts: Vec<&str> = prefix.split('/').collect();
            path_parts.extend_from_slice(&[
                "containers",
                &idx_str,
                "resources",
                "limits",
                "memory",
            ]);
            patches.push(PatchOperation::Add(AddOperation {
                path: PointerBuf::from_tokens(path_parts),
                value: Value::String(config.default_memory_limit.clone()),
            }));
        }
    }
}

fn parse_cpu_millicores(value: &str) -> Option<u64> {
    if let Some(millis) = value.strip_suffix('m') {
        millis.parse::<f64>().ok().map(|v| v as u64)
    } else {
        value.parse::<f64>().ok().map(|v| (v * 1000.0) as u64)
    }
}

fn parse_memory_bytes(value: &str) -> Option<u64> {
    if let Some(n) = value.strip_suffix("Gi") {
        n.parse::<u64>().ok().map(|v| v * 1024 * 1024 * 1024)
    } else if let Some(n) = value.strip_suffix("Mi") {
        n.parse::<u64>().ok().map(|v| v * 1024 * 1024)
    } else if let Some(n) = value.strip_suffix("Ki") {
        n.parse::<u64>().ok().map(|v| v * 1024)
    } else if let Some(n) = value.strip_suffix('G') {
        n.parse::<u64>().ok().map(|v| v * 1_000_000_000)
    } else if let Some(n) = value.strip_suffix('M') {
        n.parse::<u64>().ok().map(|v| v * 1_000_000)
    } else if let Some(n) = value.strip_suffix('k') {
        n.parse::<u64>().ok().map(|v| v * 1_000)
    } else {
        value.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cpu_millicores() {
        assert_eq!(parse_cpu_millicores("100m"), Some(100));
        assert_eq!(parse_cpu_millicores("1"), Some(1000));
        assert_eq!(parse_cpu_millicores("0.5"), Some(500));
        assert_eq!(parse_cpu_millicores("1.5"), Some(1500));
        assert_eq!(parse_cpu_millicores("250m"), Some(250));
    }

    #[test]
    fn test_parse_memory_bytes() {
        assert_eq!(parse_memory_bytes("128Mi"), Some(128 * 1024 * 1024));
        assert_eq!(parse_memory_bytes("1Gi"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_memory_bytes("512Ki"), Some(512 * 1024));
        assert_eq!(parse_memory_bytes("1000"), Some(1000));
        assert_eq!(parse_memory_bytes("1G"), Some(1_000_000_000));
        assert_eq!(parse_memory_bytes("500M"), Some(500_000_000));
    }
}
