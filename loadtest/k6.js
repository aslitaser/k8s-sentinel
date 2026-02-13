import http from "k6/http";
import { check, sleep } from "k6";
import { Rate, Trend } from "k6/metrics";
import { SharedArray } from "k6/data";

// ---------------------------------------------------------------------------
// Custom metrics
// ---------------------------------------------------------------------------
const errorRate = new Rate("errors");
const mutateLatency = new Trend("mutate_latency", true);
const validateLatency = new Trend("validate_latency", true);

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------
const BASE_URL = __ENV.TARGET_URL || "https://localhost:8443";

// ---------------------------------------------------------------------------
// Payloads — loaded once and shared across VUs (memory-efficient)
// ---------------------------------------------------------------------------
const payloads = new SharedArray("admission-reviews", function () {
  return [
    // 0: valid pod — should be allowed with no mutations
    JSON.stringify(
      JSON.parse(open("./payloads/valid_pod.json"))
    ),
    // 1: pod without resource limits — triggers resource_limits policy
    JSON.stringify(
      JSON.parse(open("./payloads/pod_no_limits.json"))
    ),
    // 2: pod with disallowed image registry — triggers image_registry policy
    JSON.stringify(
      JSON.parse(open("./payloads/pod_bad_image.json"))
    ),
    // 3: deployment with 3 containers, one violating registry policy
    JSON.stringify(
      JSON.parse(open("./payloads/deployment_multi_container.json"))
    ),
  ];
});

// ---------------------------------------------------------------------------
// Scenarios
//
// Each scenario targets a specific load profile. Run individual scenarios
// with: k6 run --env SCENARIO=spike loadtest/k6.js
// Run all (default): k6 run loadtest/k6.js
// ---------------------------------------------------------------------------
const allScenarios = {
  // Warm-up: low steady load to prime caches and connections
  baseline: {
    executor: "constant-arrival-rate",
    rate: 10,
    timeUnit: "1s",
    duration: "2m",
    preAllocatedVUs: 10,
    maxVUs: 30,
    exec: "webhookTest",
    startTime: "0s",
  },

  // Normal operating load
  normal: {
    executor: "constant-arrival-rate",
    rate: 100,
    timeUnit: "1s",
    duration: "5m",
    preAllocatedVUs: 50,
    maxVUs: 200,
    exec: "webhookTest",
    startTime: "2m",
  },

  // Spike: ramp up, hold, ramp down
  spike: {
    executor: "ramping-arrival-rate",
    startRate: 100,
    timeUnit: "1s",
    preAllocatedVUs: 200,
    maxVUs: 1500,
    stages: [
      { target: 1000, duration: "1m" },  // ramp 100 → 1000 RPS
      { target: 1000, duration: "2m" },  // hold 1000 RPS
      { target: 100, duration: "30s" },  // ramp down
    ],
    exec: "webhookTest",
    startTime: "7m",
  },

  // Soak: sustained load to detect memory leaks or degradation
  soak: {
    executor: "constant-arrival-rate",
    rate: 200,
    timeUnit: "1s",
    duration: "15m",
    preAllocatedVUs: 100,
    maxVUs: 400,
    exec: "webhookTest",
    startTime: "10m30s",
  },
};

// Allow running a single scenario via env var
const selected = __ENV.SCENARIO;
let scenarios;
let thresholds;

if (selected && allScenarios[selected]) {
  const s = allScenarios[selected];
  s.startTime = "0s"; // reset offset when running solo
  scenarios = { [selected]: s };
  thresholds = {
    http_req_duration: ["p(95)<10", "p(99)<50"],
    errors: ["rate<0.001"],
    http_reqs: ["rate>0"],
  };
} else {
  scenarios = allScenarios;
  thresholds = {
    // Latency: p95 < 10ms, p99 < 50ms
    http_req_duration: ["p(95)<10", "p(99)<50"],

    // Error rate < 0.1%
    errors: ["rate<0.001"],

    // Verify we're actually generating load (sanity check)
    http_reqs: ["rate>0"],

    // Per-endpoint latency tracking
    mutate_latency: ["p(95)<10", "p(99)<50"],
    validate_latency: ["p(95)<10", "p(99)<50"],
  };
}

export const options = {
  scenarios: scenarios,
  tlsAuth: [],
  insecureSkipTLSVerify: true,
  thresholds: thresholds,
  summaryTrendStats: [
    "avg",
    "min",
    "med",
    "max",
    "p(90)",
    "p(95)",
    "p(99)",
  ],
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
function randomPayload() {
  return payloads[Math.floor(Math.random() * payloads.length)];
}

function makeParams(endpoint) {
  return {
    headers: { "Content-Type": "application/json" },
    tags: { endpoint: endpoint },
  };
}

// Stamp each request with a unique UID so the webhook doesn't deduplicate
function stampUID(payload) {
  const uid =
    `${Date.now()}-${Math.random().toString(36).substring(2, 10)}`;
  return payload.replace(
    /a1b2c3d4-0000-0000-0000-00000000000\d/,
    uid
  );
}

// ---------------------------------------------------------------------------
// Test function — called by all scenarios
// ---------------------------------------------------------------------------
export function webhookTest() {
  const body = stampUID(randomPayload());

  // Alternate between /mutate and /validate
  if (Math.random() < 0.5) {
    const res = http.post(`${BASE_URL}/mutate`, body, makeParams("mutate"));
    mutateLatency.add(res.timings.duration);

    const ok = check(res, {
      "mutate: status is 200": (r) => r.status === 200,
      "mutate: has apiVersion": (r) => {
        try {
          return JSON.parse(r.body).apiVersion === "admission.k8s.io/v1";
        } catch (_) {
          return false;
        }
      },
      "mutate: has response uid": (r) => {
        try {
          return JSON.parse(r.body).response.uid !== undefined;
        } catch (_) {
          return false;
        }
      },
    });
    errorRate.add(!ok);
  } else {
    const res = http.post(
      `${BASE_URL}/validate`,
      body,
      makeParams("validate")
    );
    validateLatency.add(res.timings.duration);

    const ok = check(res, {
      "validate: status is 200": (r) => r.status === 200,
      "validate: has apiVersion": (r) => {
        try {
          return JSON.parse(r.body).apiVersion === "admission.k8s.io/v1";
        } catch (_) {
          return false;
        }
      },
      "validate: has response uid": (r) => {
        try {
          return JSON.parse(r.body).response.uid !== undefined;
        } catch (_) {
          return false;
        }
      },
    });
    errorRate.add(!ok);
  }
}

// ---------------------------------------------------------------------------
// Lifecycle hooks
// ---------------------------------------------------------------------------
export function setup() {
  // Verify the server is reachable before starting the load test
  const res = http.get(`${BASE_URL.replace("8443", "9090")}/healthz`, {
    timeout: "5s",
  });
  check(res, {
    "setup: server is healthy": (r) => r.status === 200,
  });
  console.log(`Target: ${BASE_URL}`);
  console.log(`Payloads: ${payloads.length} admission review variants`);
}

export function teardown(data) {
  console.log("Load test complete.");
}
