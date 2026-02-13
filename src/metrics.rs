use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::Histogram;
use prometheus_client::registry::Registry;

use crate::config::{PoliciesConfig, PolicyName};

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct RequestLabels {
    pub operation: String,
    pub resource: String,
    pub webhook: &'static str,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ResponseLabels {
    pub allowed: &'static str,
    pub webhook: &'static str,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct PolicyEvalLabels {
    pub policy: &'static str,
    pub result: &'static str,
    pub mode: &'static str,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct WebhookLabels {
    pub webhook: &'static str,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct PolicyLabels {
    pub policy: &'static str,
}

pub struct SentinelMetrics {
    pub admission_requests_total: Family<RequestLabels, Counter>,
    pub admission_responses_total: Family<ResponseLabels, Counter>,
    pub policy_evaluations_total: Family<PolicyEvalLabels, Counter>,
    pub admission_request_duration_seconds: Family<WebhookLabels, Histogram>,
    pub policy_evaluation_duration_seconds: Family<PolicyLabels, Histogram>,
}

const DURATION_BUCKETS: [f64; 14] = [
    0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

fn new_duration_histogram() -> Histogram {
    Histogram::new(DURATION_BUCKETS.iter().copied())
}

impl SentinelMetrics {
    pub fn new(registry: &mut Registry, policies_config: &PoliciesConfig) -> Self {
        let admission_requests_total = Family::<RequestLabels, Counter>::default();
        registry.register(
            "sentinel_admission_requests",
            "Total number of admission requests received",
            admission_requests_total.clone(),
        );

        let admission_responses_total = Family::<ResponseLabels, Counter>::default();
        registry.register(
            "sentinel_admission_responses",
            "Total number of admission responses sent",
            admission_responses_total.clone(),
        );

        let policy_evaluations_total = Family::<PolicyEvalLabels, Counter>::default();
        registry.register(
            "sentinel_policy_evaluations",
            "Total number of policy evaluations",
            policy_evaluations_total.clone(),
        );

        let admission_request_duration_seconds =
            Family::<WebhookLabels, Histogram>::new_with_constructor(new_duration_histogram);
        registry.register(
            "sentinel_admission_request_duration_seconds",
            "Duration of admission request processing in seconds",
            admission_request_duration_seconds.clone(),
        );

        let policy_evaluation_duration_seconds =
            Family::<PolicyLabels, Histogram>::new_with_constructor(new_duration_histogram);
        registry.register(
            "sentinel_policy_evaluation_duration_seconds",
            "Duration of individual policy evaluations in seconds",
            policy_evaluation_duration_seconds.clone(),
        );

        let policies_enabled = Family::<PolicyLabels, Gauge>::default();
        registry.register(
            "sentinel_policies_enabled",
            "Whether each policy is enabled (1) or disabled (0)",
            policies_enabled.clone(),
        );

        for name in PolicyName::ALL {
            policies_enabled
                .get_or_create(&PolicyLabels {
                    policy: name.as_str(),
                })
                .set(if policies_config.policy_enabled(name) { 1 } else { 0 });
        }

        Self {
            admission_requests_total,
            admission_responses_total,
            policy_evaluations_total,
            admission_request_duration_seconds,
            policy_evaluation_duration_seconds,
        }
    }
}
