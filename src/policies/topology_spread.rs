use json_patch::jsonptr::PointerBuf;
use json_patch::{AddOperation, PatchOperation};
use kube::core::admission::AdmissionRequest;
use kube::core::DynamicObject;
use serde_json::{json, Value};

use crate::config::TopologySpreadPolicy;

use super::{get_pod_spec, spec_prefix, PolicyOutput};

pub fn evaluate(
    config: &TopologySpreadPolicy,
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

    let resource_name = super::resource_name(request, object);

    let prefix = spec_prefix(kind);
    let constraints = pod_spec
        .get("topologySpreadConstraints")
        .and_then(|c| c.as_array());

    let mut violations = Vec::new();
    let mut patches = Vec::new();

    match constraints {
        Some(constraints) if !constraints.is_empty() => {
            for (i, constraint) in constraints.iter().enumerate() {
                if let Some(max_skew) = constraint.get("maxSkew").and_then(|v| v.as_i64()) {
                    if max_skew > config.max_skew as i64 {
                        let topology_key = constraint
                            .get("topologyKey")
                            .and_then(|v| v.as_str())
                            .unwrap_or("<unset>");
                        violations.push(format!(
                            "topologySpreadConstraints[{i}] on {} '{}' has maxSkew={} \
                             (topologyKey='{topology_key}') exceeding maximum {}",
                            kind,
                            resource_name,
                            max_skew,
                            config.max_skew,
                        ));
                    }
                }
            }
        }
        _ => {
            // Skip violation in mutate path if inject_if_missing will fix it
            let will_be_patched = mutating && config.inject_if_missing;
            if !will_be_patched {
                violations.push(format!(
                    "{kind} '{resource_name}' has no topologySpreadConstraints"
                ));
            }

            if config.inject_if_missing {
                let label_selector = build_label_selector(object, kind);

                let constraint = json!([{
                    "maxSkew": config.max_skew,
                    "topologyKey": config.topology_key,
                    "whenUnsatisfiable": config.when_unsatisfiable,
                    "labelSelector": label_selector,
                }]);

                let mut path_parts: Vec<&str> = prefix.split('/').collect();
                path_parts.push("topologySpreadConstraints");
                patches.push(PatchOperation::Add(AddOperation {
                    path: PointerBuf::from_tokens(path_parts),
                    value: constraint,
                }));
            }
        }
    }

    PolicyOutput {
        violations,
        patches,
    }
}

fn build_label_selector(object: &DynamicObject, kind: &str) -> Value {
    let labels = get_pod_labels(object, kind);
    json!({ "matchLabels": labels })
}

fn get_pod_labels(object: &DynamicObject, kind: &str) -> Value {
    match kind {
        "Pod" => match &object.metadata.labels {
            Some(labels) if !labels.is_empty() => {
                let map: serde_json::Map<String, Value> = labels
                    .iter()
                    .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                    .collect();
                Value::Object(map)
            }
            _ => json!({}),
        },
        _ => object
            .data
            .get("spec")
            .and_then(|s| s.get("template"))
            .and_then(|t| t.get("metadata"))
            .and_then(|m| m.get("labels"))
            .cloned()
            .unwrap_or(json!({})),
    }
}
