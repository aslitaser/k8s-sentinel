use kube::core::admission::AdmissionRequest;
use kube::core::DynamicObject;
use regex::Regex;
use tracing::warn;

use crate::config::RequiredLabelsPolicy;

use super::PolicyOutput;

pub struct CompiledLabel {
    pub key: String,
    pub pattern: Option<Regex>,
}

pub fn compile_labels(config: &RequiredLabelsPolicy) -> Vec<CompiledLabel> {
    config
        .labels
        .iter()
        .map(|label| CompiledLabel {
            key: label.key.clone(),
            pattern: label.pattern.as_ref().map(|p| {
                Regex::new(p).unwrap_or_else(|e| {
                    warn!(
                        pattern = %p,
                        key = %label.key,
                        "invalid regex pattern for required label, \
                         falling back to literal match: {e}"
                    );
                    Regex::new(&regex::escape(p)).unwrap()
                })
            }),
        })
        .collect()
}

pub fn evaluate(
    compiled_labels: &[CompiledLabel],
    request: &AdmissionRequest<DynamicObject>,
) -> PolicyOutput {
    let object = match &request.object {
        Some(obj) => obj,
        None => return PolicyOutput::allowed(),
    };

    let labels = object.metadata.labels.as_ref();
    let resource_name = super::resource_name(request, object);

    let mut violations = Vec::new();

    for cl in compiled_labels {
        match labels.and_then(|l| l.get(&cl.key)) {
            None => {
                violations.push(format!(
                    "missing required label '{}' on {} '{}'",
                    cl.key, request.kind.kind, resource_name,
                ));
            }
            Some(value) => {
                if let Some(pattern) = &cl.pattern {
                    if !pattern.is_match(value) {
                        violations.push(format!(
                            "label '{}' on {} '{}' has value '{}' which does not match \
                             required pattern '{}'",
                            cl.key, request.kind.kind, resource_name, value, pattern.as_str(),
                        ));
                    }
                }
            }
        }
    }

    PolicyOutput {
        violations,
        patches: Vec::new(),
    }
}
