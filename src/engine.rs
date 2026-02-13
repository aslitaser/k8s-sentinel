use std::time::{Duration, Instant};

use json_patch::PatchOperation;
use kube::core::admission::AdmissionRequest;
use kube::core::DynamicObject;

use crate::config::{PoliciesConfig, PolicyMode, PolicyName};
use crate::policies::labels::CompiledLabel;
use crate::policies::{self, PolicyOutput};

pub struct PolicyResult {
    pub policy_name: PolicyName,
    pub allowed: bool,
    pub message: Option<String>,
    pub warnings: Vec<String>,
    pub patches: Vec<PatchOperation>,
    pub duration: Duration,
}

pub struct PolicyEngine {
    pub config: PoliciesConfig,
    compiled_labels: Vec<CompiledLabel>,
}

impl PolicyEngine {
    pub fn new(config: PoliciesConfig) -> Self {
        let compiled_labels = policies::labels::compile_labels(&config.labels);
        Self {
            config,
            compiled_labels,
        }
    }

    pub fn evaluate_validate(
        &self,
        request: &AdmissionRequest<DynamicObject>,
    ) -> Vec<PolicyResult> {
        self.evaluate_all(request, false)
    }

    pub fn evaluate_mutate(
        &self,
        request: &AdmissionRequest<DynamicObject>,
    ) -> Vec<PolicyResult> {
        self.evaluate_all(request, true)
    }

    fn evaluate_all(
        &self,
        request: &AdmissionRequest<DynamicObject>,
        include_patches: bool,
    ) -> Vec<PolicyResult> {
        PolicyName::ALL
            .iter()
            .filter(|name| self.config.policy_enabled(**name))
            .map(|&name| {
                let start = Instant::now();
                let output = match name {
                    PolicyName::ResourceLimits => policies::resource_limits::evaluate(
                        &self.config.resource_limits,
                        request,
                        include_patches,
                    ),
                    PolicyName::ImageRegistry => {
                        policies::image_registry::evaluate(&self.config.image_registry, request)
                    }
                    PolicyName::Labels => {
                        policies::labels::evaluate(&self.compiled_labels, request)
                    }
                    PolicyName::TopologySpread => policies::topology_spread::evaluate(
                        &self.config.topology_spread,
                        request,
                        include_patches,
                    ),
                };
                self.to_result(name, output, include_patches, start.elapsed())
            })
            .collect()
    }

    fn to_result(
        &self,
        name: PolicyName,
        output: PolicyOutput,
        include_patches: bool,
        duration: Duration,
    ) -> PolicyResult {
        let patches = if include_patches {
            output.patches
        } else {
            vec![]
        };

        match self.config.policy_mode(name) {
            PolicyMode::Enforce => PolicyResult {
                policy_name: name,
                allowed: output.violations.is_empty(),
                message: if output.violations.is_empty() {
                    None
                } else {
                    Some(output.violations.join("; "))
                },
                warnings: vec![],
                patches,
                duration,
            },
            PolicyMode::Warn => PolicyResult {
                policy_name: name,
                allowed: true,
                message: None,
                warnings: output
                    .violations
                    .into_iter()
                    .map(|v| format!("{name}: {v}"))
                    .collect(),
                patches,
                duration,
            },
        }
    }
}
