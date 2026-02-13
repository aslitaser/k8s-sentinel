use std::sync::Arc;
use std::time::Instant;

use axum::extract::State;
use axum::Json;
use json_patch::Patch;
use kube::core::admission::{AdmissionRequest, AdmissionResponse, AdmissionReview};
use kube::core::DynamicObject;
use tracing::{error, warn};

use crate::config::PolicyMode;
use crate::engine::{PolicyEngine, PolicyResult};
use crate::metrics::{
    PolicyEvalLabels, PolicyLabels, RequestLabels, ResponseLabels, SentinelMetrics, WebhookLabels,
};

pub struct AppState {
    pub engine: PolicyEngine,
    pub metrics: SentinelMetrics,
}

pub type SharedState = Arc<AppState>;

#[derive(Clone, Copy)]
enum WebhookType {
    Validate,
    Mutate,
}

impl WebhookType {
    fn as_str(self) -> &'static str {
        match self {
            WebhookType::Validate => "validate",
            WebhookType::Mutate => "mutate",
        }
    }
}

pub async fn handle_validate(
    state: State<SharedState>,
    body: Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    handle_webhook(state, body, WebhookType::Validate)
}

pub async fn handle_mutate(
    state: State<SharedState>,
    body: Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    handle_webhook(state, body, WebhookType::Mutate)
}

fn handle_webhook(
    State(state): State<SharedState>,
    body: Json<serde_json::Value>,
    webhook_type: WebhookType,
) -> Json<serde_json::Value> {
    let start = Instant::now();
    let wh = webhook_type.as_str();

    let review: AdmissionReview<DynamicObject> = match serde_json::from_value(body.0) {
        Ok(r) => r,
        Err(e) => {
            warn!("failed to deserialize AdmissionReview: {e}");
            let resp = AdmissionResponse::invalid(format!("failed to deserialize request: {e}"));
            return review_to_json(resp.into_review());
        }
    };

    let req: AdmissionRequest<DynamicObject> = match review.try_into() {
        Ok(r) => r,
        Err(e) => {
            warn!("AdmissionReview missing request field: {e}");
            let resp = AdmissionResponse::invalid("missing request field in AdmissionReview");
            return review_to_json(resp.into_review());
        }
    };

    record_request_metrics(&state, &req, wh);

    let evaluate = match webhook_type {
        WebhookType::Validate => PolicyEngine::evaluate_validate,
        WebhookType::Mutate => PolicyEngine::evaluate_mutate,
    };

    let results = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        evaluate(&state.engine, &req)
    }));

    let results = match results {
        Ok(r) => r,
        Err(_) => {
            error!(uid = %req.uid, "policy evaluation panicked, failing open");
            record_response_metrics(&state, true, wh);
            let mut resp = AdmissionResponse::from(&req);
            resp.warnings = Some(vec![
                "sentinel: internal error during policy evaluation, failing open".to_string(),
            ]);
            observe_request_duration(&state, wh, start);
            return review_to_json(resp.into_review());
        }
    };

    record_policy_eval_metrics(&state, &results);
    let response = build_response(&req, &results, webhook_type);
    record_response_metrics(&state, response.allowed, wh);
    observe_request_duration(&state, wh, start);

    review_to_json(response.into_review())
}

fn review_to_json(review: AdmissionReview<DynamicObject>) -> Json<serde_json::Value> {
    Json(serde_json::to_value(review).expect("AdmissionReview serialization is infallible"))
}

fn build_response(
    req: &AdmissionRequest<DynamicObject>,
    results: &[PolicyResult],
    webhook_type: WebhookType,
) -> AdmissionResponse {
    let mut resp = AdmissionResponse::from(req);
    let mut warnings: Vec<String> = Vec::new();
    let mut denied_messages: Vec<String> = Vec::new();
    let mut all_patches: Vec<json_patch::PatchOperation> = Vec::new();

    for result in results {
        warnings.extend(result.warnings.iter().cloned());

        if matches!(webhook_type, WebhookType::Mutate) {
            all_patches.extend(result.patches.iter().cloned());
        }

        if !result.allowed {
            if let Some(msg) = &result.message {
                denied_messages.push(format!("{}: {msg}", result.policy_name));
            } else {
                denied_messages.push(format!("{}: denied", result.policy_name));
            }
        }
    }

    if !denied_messages.is_empty() {
        resp = resp.deny(denied_messages.join("; "));
    } else if !all_patches.is_empty() {
        resp = match resp.with_patch(Patch(all_patches)) {
            Ok(patched) => patched,
            Err(e) => {
                error!("failed to serialize patches: {e}");
                let mut fallback = AdmissionResponse::from(req);
                fallback.warnings = Some(vec![
                    "sentinel: failed to serialize patches".to_string(),
                ]);
                fallback
            }
        };
    }

    if !warnings.is_empty() {
        resp.warnings.get_or_insert_with(Vec::new).extend(warnings);
    }

    resp
}

fn record_request_metrics(
    state: &AppState,
    req: &AdmissionRequest<DynamicObject>,
    webhook: &'static str,
) {
    let operation = format!("{:?}", req.operation).to_uppercase();
    let resource = req.resource.resource.clone();

    state
        .metrics
        .admission_requests_total
        .get_or_create(&RequestLabels {
            operation,
            resource,
            webhook,
        })
        .inc();
}

fn record_response_metrics(state: &AppState, allowed: bool, webhook: &'static str) {
    state
        .metrics
        .admission_responses_total
        .get_or_create(&ResponseLabels {
            allowed: if allowed { "true" } else { "false" },
            webhook,
        })
        .inc();
}

fn record_policy_eval_metrics(state: &AppState, results: &[PolicyResult]) {
    for result in results {
        let mode = if result.allowed && result.warnings.is_empty() {
            mode_str(state.engine.config.policy_mode(result.policy_name))
        } else if !result.allowed {
            "enforce"
        } else {
            "warn"
        };

        let eval_result = if !result.allowed {
            "denied"
        } else if !result.warnings.is_empty() {
            "warning"
        } else {
            "allowed"
        };

        state
            .metrics
            .policy_evaluations_total
            .get_or_create(&PolicyEvalLabels {
                policy: result.policy_name.as_str(),
                result: eval_result,
                mode,
            })
            .inc();

        state
            .metrics
            .policy_evaluation_duration_seconds
            .get_or_create(&PolicyLabels {
                policy: result.policy_name.as_str(),
            })
            .observe(result.duration.as_secs_f64());
    }
}

fn observe_request_duration(state: &AppState, webhook: &'static str, start: Instant) {
    state
        .metrics
        .admission_request_duration_seconds
        .get_or_create(&WebhookLabels { webhook })
        .observe(start.elapsed().as_secs_f64());
}

fn mode_str(mode: &PolicyMode) -> &'static str {
    match mode {
        PolicyMode::Enforce => "enforce",
        PolicyMode::Warn => "warn",
    }
}
